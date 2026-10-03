-- Alert rules can text or call a US phone number when they trigger. The trigger only
-- queues a notification on the alert event; Vercel functions claim and send it via Twilio.
alter table public.fizz_alert_rules
  add column notify_channel text not null default 'none' check (notify_channel in ('none', 'sms', 'call')),
  add column notify_phone text check (notify_phone ~ '^\+1[2-9][0-9]{2}[2-9][0-9]{6}$'),
  add constraint fizz_alert_rules_notify_target check ((notify_channel = 'none') = (notify_phone is null));

alter table public.fizz_alert_events
  add column notify_channel text not null default 'none' check (notify_channel in ('none', 'sms', 'call')),
  add column notify_phone text,
  add column notify_status text not null default 'none'
    check (notify_status in ('none', 'pending', 'sending', 'sent', 'failed', 'skipped')),
  add column notify_attempts smallint not null default 0,
  add column notify_claimed_at timestamptz,
  add column notified_at timestamptz,
  add column notify_error text;
create index fizz_alert_events_notify_queue_idx on public.fizz_alert_events(created_at)
  where notify_status in ('pending', 'sending');

create or replace function public.fizz_evaluate_alerts() returns trigger
language plpgsql security definer set search_path = '' as $$
declare
  v_rule public.fizz_alert_rules%rowtype;
  v_hit boolean;
  v_notify text;
begin
  if new.numeric_value is null then return new; end if;
  for v_rule in
    select * from public.fizz_alert_rules
    where sensor_id = new.sensor_id and metric = new.metric and enabled
    for update
  loop
    if v_rule.unit <> coalesce(new.unit, '') then continue; end if;
    v_hit := case v_rule.comparator
      when 'gt' then new.numeric_value > v_rule.threshold
      when 'gte' then new.numeric_value >= v_rule.threshold
      when 'lt' then new.numeric_value < v_rule.threshold
      else new.numeric_value <= v_rule.threshold end;
    if v_hit and v_rule.armed then
      v_notify := 'none';
      if v_rule.notify_channel <> 'none' then
        -- A per-rule cooldown and an hourly per-customer cap bound texting and calling.
        if exists(select 1 from public.fizz_alert_events
            where rule_id = v_rule.id and notify_status in ('pending', 'sending', 'sent')
              and created_at > now() - interval '5 minutes')
          or (select count(*) from public.fizz_alert_events
            where customer_id = v_rule.customer_id and notify_status in ('pending', 'sending', 'sent')
              and created_at > now() - interval '1 hour') >= 20 then
          v_notify := 'skipped';
        else
          v_notify := 'pending';
        end if;
      end if;
      insert into public.fizz_alert_events
        (rule_id, reading_id, customer_id, sensor_id, metric, numeric_value, threshold, comparator, unit, observed_at,
         notify_channel, notify_phone, notify_status)
      values
        (v_rule.id, new.id, v_rule.customer_id, new.sensor_id, new.metric,
         new.numeric_value, v_rule.threshold, v_rule.comparator, v_rule.unit, new.observed_at,
         v_rule.notify_channel, v_rule.notify_phone, v_notify)
      on conflict (rule_id, reading_id) do nothing;
      update public.fizz_alert_rules set armed = false, updated_at = now() where id = v_rule.id;
    elsif not v_hit and not v_rule.armed then
      update public.fizz_alert_rules set armed = true, updated_at = now() where id = v_rule.id;
    end if;
  end loop;
  return new;
end;
$$;

drop function public.fizz_create_alert(text, uuid, text, text, double precision, text);
create function public.fizz_create_alert(p_token text, p_sensor_id uuid, p_metric text,
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
  if coalesce(p_notify_channel, 'none') not in ('none','sms','call')
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

-- Claim a small batch of queued notifications. Abandoned sends are retried after two
-- minutes, at most three attempts, and anything older than an hour expires.
create function public.fizz_claim_alert_notifications(p_limit integer default 3)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_batch jsonb;
begin
  update public.fizz_alert_events
  set notify_status = 'failed', notify_error = 'Expired before it could be sent.'
  where notify_status in ('pending', 'sending')
    and (created_at < now() - interval '1 hour'
      or (notify_status = 'sending' and notify_attempts >= 3 and notify_claimed_at < now() - interval '2 minutes'));
  with claimed as (
    update public.fizz_alert_events e
    set notify_status = 'sending', notify_claimed_at = now(), notify_attempts = e.notify_attempts + 1
    where e.id in (
      select id from public.fizz_alert_events
      where (notify_status = 'pending'
          or (notify_status = 'sending' and notify_claimed_at < now() - interval '2 minutes'))
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

create function public.fizz_finish_alert_notification(p_event_id uuid, p_sent boolean,
  p_retry boolean default false, p_error text default null)
returns void language sql security definer set search_path = '' as $$
  update public.fizz_alert_events set
    notify_status = case when p_sent then 'sent'
      when p_retry and notify_attempts < 3 then 'pending' else 'failed' end,
    notified_at = case when p_sent then now() end,
    notify_error = case when p_sent then null else left(p_error, 200) end
  where id = p_event_id and notify_status = 'sending';
$$;

revoke execute on function public.fizz_create_alert(text,uuid,text,text,double precision,text,text,text),
  public.fizz_claim_alert_notifications(integer),
  public.fizz_finish_alert_notification(uuid,boolean,boolean,text)
  from public, anon, authenticated;
grant execute on function public.fizz_create_alert(text,uuid,text,text,double precision,text,text,text),
  public.fizz_claim_alert_notifications(integer),
  public.fizz_finish_alert_notification(uuid,boolean,boolean,text)
  to service_role;
