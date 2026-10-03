-- Preserve latest values and consented phone transcripts even when frequent motion
-- readings fill the bounded recent-reading window.
create or replace function public.fizz_chat_context(p_token text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  return jsonb_build_object(
    'sensors', coalesce((select jsonb_agg(jsonb_build_object('id',s.id,'name',s.name,'kind',s.kind,'mode',s.mode))
      from public.fizz_sensors s where s.customer_id = v_customer and s.deleted_at is null), '[]'::jsonb),
    'latest', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',q.sensor_id,'metric',q.metric,
      'numeric_value',q.numeric_value,'boolean_value',q.boolean_value,'text_value',q.text_value,
      'unit',q.unit,'observed_at',q.observed_at,'source',q.source))
      from (select * from (select distinct on (r.sensor_id,r.metric) r.*
        from public.fizz_sensor_readings r join public.fizz_sensors s on s.id = r.sensor_id
        where s.customer_id = v_customer and s.deleted_at is null
        order by r.sensor_id,r.metric,r.observed_at desc,r.received_at desc) latest_rows
        order by observed_at desc limit 100) q), '[]'::jsonb),
    'readings', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',q.sensor_id,'metric',q.metric,
      'numeric_value',q.numeric_value,'boolean_value',q.boolean_value,'text_value',q.text_value,
      'unit',q.unit,'observed_at',q.observed_at,'source',q.source))
      from (select r.* from public.fizz_sensor_readings r join public.fizz_sensors s on s.id = r.sensor_id
        where s.customer_id = v_customer and s.deleted_at is null
        order by r.observed_at desc limit 100) q), '[]'::jsonb),
    'transcripts', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',q.sensor_id,
      'text_value',q.text_value,'observed_at',q.observed_at))
      from (select r.* from public.fizz_sensor_readings r join public.fizz_sensors s on s.id = r.sensor_id
        where s.customer_id = v_customer and s.deleted_at is null and r.metric = 'voice_transcript'
        order by r.observed_at desc limit 20) q), '[]'::jsonb),
    'rules', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',r.sensor_id,'metric',r.metric,
      'comparator',r.comparator,'threshold',r.threshold,'unit',r.unit,'enabled',r.enabled))
      from public.fizz_alert_rules r join public.fizz_sensors s on s.id = r.sensor_id
      where r.customer_id = v_customer and s.deleted_at is null), '[]'::jsonb),
    'alerts', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',e.sensor_id,'metric',e.metric,
      'numeric_value',e.numeric_value,'threshold',e.threshold,'unit',e.unit,'observed_at',e.observed_at))
      from (select * from public.fizz_alert_events where customer_id = v_customer
        order by created_at desc limit 20) e), '[]'::jsonb));
end;
$$;

-- A rule tied to a removed sensor must stay disabled, including API attempts
-- to re-enable it after removal.
create or replace function public.fizz_update_alert(p_token text, p_rule_id uuid, p_enabled boolean, p_delete boolean)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_rule public.fizz_alert_rules%rowtype;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  if coalesce(p_delete,false) then
    delete from public.fizz_alert_rules where id = p_rule_id and customer_id = v_customer returning * into v_rule;
    if not found then return jsonb_build_object('error','not_found'); end if;
    return jsonb_build_object('ok',true);
  end if;
  if p_enabled is null then return jsonb_build_object('error','invalid_rule'); end if;
  if p_enabled and not exists(select 1 from public.fizz_alert_rules r
      join public.fizz_sensors s on s.id = r.sensor_id
      where r.id = p_rule_id and r.customer_id = v_customer and s.deleted_at is null) then
    return jsonb_build_object('error','not_found');
  end if;
  update public.fizz_alert_rules set enabled = p_enabled, armed = true, updated_at = now()
  where id = p_rule_id and customer_id = v_customer returning * into v_rule;
  if not found then return jsonb_build_object('error','not_found'); end if;
  return jsonb_build_object('rule',to_jsonb(v_rule)-'customer_id');
end;
$$;
