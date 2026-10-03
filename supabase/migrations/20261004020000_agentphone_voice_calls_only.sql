-- Phone notifications currently support voice calls only. Preserve old SMS
-- events as history, while retaining their rules as dashboard-only alerts.
update public.fizz_alert_rules
set notify_channel = 'none', notify_phone = null, updated_at = now()
where notify_channel = 'sms';

update public.fizz_alert_events
set notify_status = 'skipped', notify_error = 'SMS notifications are disabled; only voice calls are enabled.'
where notify_channel = 'sms' and notify_status in ('pending', 'sending');

alter table public.fizz_alert_rules
  drop constraint fizz_alert_rules_notify_channel_check,
  add constraint fizz_alert_rules_notify_channel_check check (notify_channel in ('none', 'call'));

create or replace function public.fizz_create_alert(p_token text, p_sensor_id uuid, p_metric text,
  p_comparator text, p_threshold double precision, p_unit text,
  p_notify_channel text default 'none', p_notify_phone text default null)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_rule public.fizz_alert_rules%rowtype; v_unit text;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  if not exists(select 1 from public.fizz_sensors where id = p_sensor_id
      and customer_id = v_customer and deleted_at is null) then
    return jsonb_build_object('error','sensor_not_found');
  end if;
  if p_metric is null or p_metric !~ '^[a-z][a-z0-9_]{0,63}$'
    or p_comparator not in ('gt','gte','lt','lte')
    or p_threshold is null or p_threshold = 'Infinity'::float8 or p_threshold = '-Infinity'::float8
    or p_unit is null or length(p_unit) > 24 then
    return jsonb_build_object('error','invalid_rule');
  end if;
  if coalesce(p_notify_channel, 'none') not in ('none','call')
    or (coalesce(p_notify_channel, 'none') = 'none') <> (p_notify_phone is null)
    or p_notify_phone !~ '^\+1[2-9][0-9]{2}[2-9][0-9]{6}$' then
    return jsonb_build_object('error','invalid_notify');
  end if;
  -- A rule can only target a metric/unit the sensor has actually reported.
  select coalesce(unit, '') into v_unit from public.fizz_sensor_readings
  where sensor_id = p_sensor_id and metric = p_metric and numeric_value is not null
  order by received_at desc limit 1;
  if not found or v_unit <> p_unit then
    return jsonb_build_object('error','unknown_metric');
  end if;
  insert into public.fizz_alert_rules(customer_id,sensor_id,metric,comparator,threshold,unit,notify_channel,notify_phone)
  values(v_customer,p_sensor_id,p_metric,p_comparator,p_threshold,p_unit,coalesce(p_notify_channel,'none'),p_notify_phone)
  returning * into v_rule;
  return jsonb_build_object('rule',to_jsonb(v_rule)-'customer_id');
end;
$$;

-- Retry only explicit, safe rejections returned to pending by the sender.
create or replace function public.fizz_claim_alert_notifications(p_limit integer default 3)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_batch jsonb;
begin
  -- AgentPhone has no idempotency keys: do not repeat an abandoned call
  -- whose provider result or database acknowledgment could have been lost.
  update public.fizz_alert_events
  set notify_status = 'failed', notify_error = 'Send outcome unknown; check AgentPhone history before retrying.'
  where notify_status = 'sending' and notify_claimed_at < now() - interval '2 minutes';
  update public.fizz_alert_events
  set notify_status = 'failed', notify_error = 'Expired before it could be sent.'
  where notify_status = 'pending' and created_at < now() - interval '1 hour';
  with claimed as (
    update public.fizz_alert_events e
    set notify_status = 'sending', notify_claimed_at = now(), notify_attempts = e.notify_attempts + 1
    where e.id in (
      select id from public.fizz_alert_events
      where notify_status = 'pending' and notify_channel = 'call'
        and notify_attempts < 3
      order by created_at
      limit least(greatest(coalesce(p_limit, 3), 1), 10)
      for update skip locked)
    returning e.*)
  select coalesce(jsonb_agg(jsonb_build_object('id', c.id, 'channel', c.notify_channel, 'phone', c.notify_phone,
      'sensor_name', s.name, 'metric', c.metric, 'numeric_value', c.numeric_value, 'threshold', c.threshold,
      'comparator', c.comparator, 'unit', c.unit, 'observed_at', c.observed_at)), '[]'::jsonb)
  into v_batch
  from claimed c join public.fizz_sensors s on s.id = c.sensor_id;
  return v_batch;
end;
$$;

