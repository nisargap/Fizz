-- Chat creates customer-scoped alerts directly. Repeated requests reuse an
-- identical active rule, including after a response is lost or a double submit.
create function public.fizz_create_chat_alert(p_token text, p_sensor_id uuid, p_metric text,
  p_comparator text, p_threshold double precision, p_unit text,
  p_notify_channel text default 'none', p_notify_phone text default null)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_rule public.fizz_alert_rules%rowtype; v_result jsonb;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  -- Serialize chat alert creation per customer so concurrent repeats cannot
  -- create duplicate active rules. This lock is held only for this transaction.
  perform pg_advisory_xact_lock(hashtextextended(v_customer::text, 0));
  select r.* into v_rule from public.fizz_alert_rules r
  join public.fizz_sensors s on s.id = r.sensor_id
  where r.customer_id = v_customer and s.deleted_at is null and r.enabled
    and r.sensor_id = p_sensor_id and r.metric = p_metric
    and r.comparator = p_comparator and r.threshold = p_threshold and r.unit = p_unit
    and r.notify_channel = coalesce(p_notify_channel, 'none')
    and r.notify_phone is not distinct from p_notify_phone
  limit 1;
  if found then
    return jsonb_build_object('rule',to_jsonb(v_rule)-'customer_id','created',false);
  end if;
  v_result := public.fizz_create_alert(p_token, p_sensor_id, p_metric,
    p_comparator, p_threshold, p_unit, p_notify_channel, p_notify_phone);
  if v_result ? 'error' then return v_result; end if;
  return v_result || jsonb_build_object('created',true);
end;
$$;
revoke execute on function public.fizz_create_chat_alert(text,uuid,text,text,double precision,text,text,text)
  from public, anon, authenticated;
grant execute on function public.fizz_create_chat_alert(text,uuid,text,text,double precision,text,text,text)
  to service_role;
