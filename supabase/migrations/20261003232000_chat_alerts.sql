create table public.fizz_alert_rules (
  id uuid primary key default gen_random_uuid(),
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  sensor_id uuid not null references public.fizz_sensors(id),
  metric text not null check (metric ~ '^[a-z][a-z0-9_]{0,63}$'),
  comparator text not null check (comparator in ('gt', 'gte', 'lt', 'lte')),
  threshold double precision not null check (threshold > '-Infinity'::float8 and threshold < 'Infinity'::float8),
  unit text not null default '',
  enabled boolean not null default true,
  armed boolean not null default true,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create index fizz_alert_rules_sensor_metric_idx on public.fizz_alert_rules(sensor_id, metric) where enabled;

create table public.fizz_alert_events (
  id uuid primary key default gen_random_uuid(),
  rule_id uuid references public.fizz_alert_rules(id) on delete set null,
  reading_id uuid not null references public.fizz_sensor_readings(id),
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  sensor_id uuid not null references public.fizz_sensors(id),
  metric text not null,
  numeric_value double precision not null,
  threshold double precision not null,
  comparator text not null,
  unit text not null,
  observed_at timestamptz not null,
  created_at timestamptz not null default now(),
  unique(rule_id, reading_id)
);
create index fizz_alert_events_customer_time_idx on public.fizz_alert_events(customer_id, created_at desc);

alter table public.fizz_alert_rules enable row level security;
alter table public.fizz_alert_events enable row level security;
revoke all on public.fizz_alert_rules, public.fizz_alert_events from anon, authenticated;
grant all on public.fizz_alert_rules, public.fizz_alert_events to service_role;

create function public.fizz_evaluate_alerts() returns trigger
language plpgsql security definer set search_path = '' as $$
declare
  v_rule public.fizz_alert_rules%rowtype;
  v_hit boolean;
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
      insert into public.fizz_alert_events
        (rule_id, reading_id, customer_id, sensor_id, metric, numeric_value, threshold, comparator, unit, observed_at)
      values
        (v_rule.id, new.id, v_rule.customer_id, new.sensor_id, new.metric,
         new.numeric_value, v_rule.threshold, v_rule.comparator, v_rule.unit, new.observed_at)
      on conflict (rule_id, reading_id) do nothing;
      update public.fizz_alert_rules set armed = false, updated_at = now() where id = v_rule.id;
    elsif not v_hit and not v_rule.armed then
      update public.fizz_alert_rules set armed = true, updated_at = now() where id = v_rule.id;
    end if;
  end loop;
  return new;
end;
$$;
create trigger fizz_evaluate_alerts_after_reading
after insert on public.fizz_sensor_readings for each row execute function public.fizz_evaluate_alerts();

create function public.fizz_disable_sensor_alerts() returns trigger
language plpgsql security definer set search_path = '' as $$
begin
  if old.deleted_at is null and new.deleted_at is not null then
    update public.fizz_alert_rules set enabled = false, updated_at = now() where sensor_id = new.id;
  end if;
  return new;
end;
$$;
create trigger fizz_disable_sensor_alerts_after_delete
after update of deleted_at on public.fizz_sensors for each row execute function public.fizz_disable_sensor_alerts();

create function public.fizz_list_alerts(p_token text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  return jsonb_build_object(
    'rules', coalesce((select jsonb_agg(to_jsonb(r) - 'customer_id' order by r.created_at desc)
      from public.fizz_alert_rules r where r.customer_id = v_customer), '[]'::jsonb),
    'events', coalesce((select jsonb_agg(to_jsonb(e) - 'customer_id' order by e.created_at desc)
      from (select * from public.fizz_alert_events where customer_id = v_customer
        order by created_at desc limit 100) e), '[]'::jsonb));
end;
$$;

create function public.fizz_create_alert(p_token text, p_sensor_id uuid, p_metric text,
  p_comparator text, p_threshold double precision, p_unit text)
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
  -- A rule can only target a metric/unit the sensor has actually reported.
  select unit into v_unit from public.fizz_sensor_readings
  where sensor_id = p_sensor_id and metric = p_metric and numeric_value is not null
  order by received_at desc limit 1;
  if v_unit is null or v_unit <> p_unit then
    return jsonb_build_object('error','unknown_metric');
  end if;
  insert into public.fizz_alert_rules(customer_id,sensor_id,metric,comparator,threshold,unit)
  values(v_customer,p_sensor_id,p_metric,p_comparator,p_threshold,p_unit) returning * into v_rule;
  return jsonb_build_object('rule',to_jsonb(v_rule)-'customer_id');
end;
$$;

create function public.fizz_update_alert(p_token text, p_rule_id uuid, p_enabled boolean, p_delete boolean)
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
  update public.fizz_alert_rules set enabled = p_enabled, armed = true, updated_at = now()
  where id = p_rule_id and customer_id = v_customer returning * into v_rule;
  if not found then return jsonb_build_object('error','not_found'); end if;
  return jsonb_build_object('rule',to_jsonb(v_rule)-'customer_id');
end;
$$;

create function public.fizz_chat_context(p_token text)
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
    'readings', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',q.sensor_id,'metric',q.metric,
      'numeric_value',q.numeric_value,'boolean_value',q.boolean_value,'text_value',q.text_value,
      'unit',q.unit,'observed_at',q.observed_at,'source',q.source))
      from (select r.* from public.fizz_sensor_readings r join public.fizz_sensors s on s.id = r.sensor_id
        where s.customer_id = v_customer and s.deleted_at is null
        order by r.observed_at desc limit 100) q), '[]'::jsonb),
    'alerts', coalesce((select jsonb_agg(jsonb_build_object('sensor_id',e.sensor_id,'metric',e.metric,
      'numeric_value',e.numeric_value,'threshold',e.threshold,'unit',e.unit,'observed_at',e.observed_at))
      from (select * from public.fizz_alert_events where customer_id = v_customer
        order by created_at desc limit 20) e), '[]'::jsonb));
end;
$$;

revoke execute on function public.fizz_evaluate_alerts(), public.fizz_disable_sensor_alerts(),
  public.fizz_list_alerts(text),
  public.fizz_create_alert(text,uuid,text,text,double precision,text),
  public.fizz_update_alert(text,uuid,boolean,boolean), public.fizz_chat_context(text)
  from public, anon, authenticated;
grant execute on function public.fizz_list_alerts(text),
  public.fizz_create_alert(text,uuid,text,text,double precision,text),
  public.fizz_update_alert(text,uuid,boolean,boolean), public.fizz_chat_context(text)
  to service_role;
