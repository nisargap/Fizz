create table public.fizz_sensors (
  id uuid primary key default gen_random_uuid(),
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  kind text not null check (kind in ('water','gas','radio','temperature','pressure','humidity','sound','phone','custom')),
  name text not null check (char_length(name) between 1 and 80),
  mode text not null default 'simulated' check (mode in ('simulated','api','phone')),
  status text not null default 'active' check (status in ('active','paused')),
  api_key_hash bytea,
  last_sample_at timestamptz,
  created_at timestamptz not null default now(),
  deleted_at timestamptz,
  check (mode <> 'phone' or kind = 'phone')
);
create index fizz_sensors_customer_idx on public.fizz_sensors(customer_id, created_at) where deleted_at is null;

insert into public.fizz_sensors(customer_id,kind,name,mode,created_at)
select c.customer_id,c.kind,initcap(c.kind) || ' sensor','simulated',c.created_at
from public.fizz_sensor_choices c
where not exists (select 1 from public.fizz_sensors s
  where s.customer_id=c.customer_id and s.kind=c.kind and s.deleted_at is null);

create table public.fizz_sensor_readings (
  id uuid primary key default gen_random_uuid(),
  sensor_id uuid not null references public.fizz_sensors(id) on delete cascade,
  event_id text not null check (char_length(event_id) between 1 and 128),
  observed_at timestamptz not null,
  received_at timestamptz not null default now(),
  metric text not null check (metric ~ '^[a-z][a-z0-9_]{0,39}$'),
  numeric_value double precision,
  boolean_value boolean,
  text_value text,
  unit text,
  source text not null check (source in ('simulated','api','phone')),
  check ((numeric_value is not null)::integer + (boolean_value is not null)::integer + (text_value is not null)::integer = 1),
  check (numeric_value is null or (numeric_value > '-Infinity'::float8 and numeric_value < 'Infinity'::float8)),
  check (text_value is null or char_length(text_value) <= 512),
  check (unit is null or char_length(unit) <= 24),
  unique(sensor_id, event_id, metric)
);
create index fizz_readings_sensor_time_idx on public.fizz_sensor_readings(sensor_id, observed_at desc);

alter table public.fizz_sensors enable row level security;
alter table public.fizz_sensor_readings enable row level security;
revoke all on public.fizz_sensors, public.fizz_sensor_readings from anon, authenticated;
grant all on public.fizz_sensors, public.fizz_sensor_readings to service_role;

create function public.fizz_session_customer(p_token text)
returns uuid language sql stable security definer set search_path = '' as $$
  select s.customer_id from public.fizz_sessions s
  where p_token ~ '^[0-9a-f]{64}$'
    and s.token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256')
    and s.expires_at > now() limit 1;
$$;

create function public.fizz_sensor_json(p_sensor public.fizz_sensors)
returns jsonb language sql stable security definer set search_path = '' as $$
  select jsonb_build_object(
    'id', p_sensor.id, 'kind', p_sensor.kind, 'name', p_sensor.name,
    'mode', p_sensor.mode, 'status', p_sensor.status,
    'created_at', p_sensor.created_at,
    'latest_reading', (select to_jsonb(r) from public.fizz_sensor_readings r
      where r.sensor_id = p_sensor.id order by r.observed_at desc, r.received_at desc limit 1)
  );
$$;

create function public.fizz_list_sensors(p_token text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  return jsonb_build_object('sensors', coalesce((
    select jsonb_agg(public.fizz_sensor_json(s) order by s.created_at, s.id)
    from public.fizz_sensors s where s.customer_id = v_customer and s.deleted_at is null
  ), '[]'::jsonb));
end;
$$;

create function public.fizz_create_sensor(p_token text, p_kind text, p_name text, p_mode text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_sensor public.fizz_sensors%rowtype; v_key text;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  if p_kind not in ('water','gas','radio','temperature','pressure','humidity','sound','phone','custom')
    or p_mode not in ('simulated','api','phone') or (p_mode = 'phone' and p_kind <> 'phone')
    or p_name is null or char_length(trim(p_name)) not between 1 and 80 then
    return jsonb_build_object('error','invalid_request');
  end if;
  if p_mode = 'api' then v_key := 'fizz_' || encode(extensions.gen_random_bytes(32), 'hex'); end if;
  insert into public.fizz_sensors(customer_id,kind,name,mode,api_key_hash)
    values(v_customer,p_kind,trim(p_name),p_mode,
      case when v_key is null then null else extensions.digest(v_key, 'sha256') end)
    returning * into v_sensor;
  return jsonb_build_object('sensor',public.fizz_sensor_json(v_sensor),'api_key',v_key);
end;
$$;

create function public.fizz_update_sensor(p_token text, p_id uuid, p_name text default null, p_mode text default null, p_status text default null, p_rotate_key boolean default false)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_sensor public.fizz_sensors%rowtype; v_key text;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  select * into v_sensor from public.fizz_sensors where id=p_id and customer_id=v_customer and deleted_at is null for update;
  if not found then return jsonb_build_object('error','not_found'); end if;
  if (p_name is not null and char_length(trim(p_name)) not between 1 and 80)
    or (p_mode is not null and (p_mode not in ('simulated','api','phone') or (p_mode='phone' and v_sensor.kind<>'phone')))
    or (p_status is not null and p_status not in ('active','paused')) then
    return jsonb_build_object('error','invalid_request');
  end if;
  if coalesce(p_mode,v_sensor.mode) = 'api' and (v_sensor.api_key_hash is null or p_rotate_key or v_sensor.mode <> 'api') then
    v_key := 'fizz_' || encode(extensions.gen_random_bytes(32), 'hex');
  end if;
  update public.fizz_sensors set
    name=coalesce(trim(p_name),name), mode=coalesce(p_mode,mode), status=coalesce(p_status,status),
    api_key_hash=case when coalesce(p_mode,mode) <> 'api' then null
      when v_key is not null then extensions.digest(v_key,'sha256') else api_key_hash end,
    last_sample_at=case when p_mode='simulated' and v_sensor.mode <> 'simulated' then null else last_sample_at end
  where id=p_id returning * into v_sensor;
  return jsonb_build_object('sensor',public.fizz_sensor_json(v_sensor),'api_key',v_key);
end;
$$;

create function public.fizz_delete_sensor(p_token text, p_id uuid)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  update public.fizz_sensors set deleted_at=now(), status='paused', api_key_hash=null
    where id=p_id and customer_id=v_customer and deleted_at is null;
  if not found then return jsonb_build_object('error','not_found'); end if;
  return jsonb_build_object('ok',true);
end;
$$;

create function public.fizz_write_reading(p_sensor_id uuid, p_event_id text, p_observed_at timestamptz,
  p_metric text, p_numeric_value double precision, p_boolean_value boolean, p_text_value text, p_unit text, p_source text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_reading public.fizz_sensor_readings%rowtype;
begin
  if p_event_id is null or char_length(p_event_id) not between 1 and 128
    or p_metric !~ '^[a-z][a-z0-9_]{0,39}$'
    or p_source not in ('simulated','api','phone')
    or p_observed_at < now() - interval '30 days' or p_observed_at > now() + interval '5 minutes'
    or (p_numeric_value is not null)::integer + (p_boolean_value is not null)::integer + (p_text_value is not null)::integer <> 1
    or (p_numeric_value is not null and (p_numeric_value <= '-Infinity'::float8 or p_numeric_value >= 'Infinity'::float8))
    or (p_text_value is not null and char_length(p_text_value) > 512)
    or (p_unit is not null and char_length(p_unit)>24) then
    return jsonb_build_object('error','invalid_reading');
  end if;
  insert into public.fizz_sensor_readings(sensor_id,event_id,observed_at,metric,numeric_value,boolean_value,text_value,unit,source)
  values(p_sensor_id,p_event_id,p_observed_at,p_metric,p_numeric_value,p_boolean_value,p_text_value,p_unit,p_source)
  on conflict(sensor_id,event_id,metric) do update set id=public.fizz_sensor_readings.id
  returning * into v_reading;
  return to_jsonb(v_reading);
end;
$$;

create function public.fizz_ingest_reading(p_sensor_id uuid, p_key text, p_event_id text, p_observed_at timestamptz,
  p_metric text, p_numeric_value double precision, p_boolean_value boolean, p_text_value text, p_unit text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_sensor public.fizz_sensors%rowtype;
begin
  select * into v_sensor from public.fizz_sensors where id=p_sensor_id and deleted_at is null and mode='api' and status='active';
  if not found or p_key is null or p_key !~ '^fizz_[0-9a-f]{64}$'
    or v_sensor.api_key_hash <> extensions.digest(p_key,'sha256') then
    return jsonb_build_object('error','unauthorized');
  end if;
  if not coalesce((case v_sensor.kind
    when 'water' then (p_metric='flow_l_min' and p_numeric_value between 0 and 200) or (p_metric='leak' and p_boolean_value is not null)
    when 'gas' then (p_metric='concentration_ppm' and p_numeric_value between 0 and 10000) or (p_metric='alarm' and p_boolean_value is not null)
    when 'radio' then p_metric='rssi_dbm' and p_numeric_value between -150 and 0
    when 'temperature' then p_metric='temperature_c' and p_numeric_value between -100 and 200
    when 'pressure' then p_metric='pressure_kpa' and p_numeric_value between 0 and 1000
    when 'humidity' then p_metric='humidity_pct' and p_numeric_value between 0 and 100
    when 'sound' then p_metric='sound_db' and p_numeric_value between 0 and 160
    when 'phone' then (p_metric='acceleration_ms2' and p_numeric_value between 0 and 300)
      or (p_metric='orientation_deg' and p_numeric_value between -360 and 360)
    when 'custom' then true else false end), false) then
    return jsonb_build_object('error','invalid_reading');
  end if;
  return public.fizz_write_reading(p_sensor_id,p_event_id,coalesce(p_observed_at,now()),p_metric,p_numeric_value,p_boolean_value,p_text_value,p_unit,'api');
end;
$$;

create function public.fizz_list_readings(p_token text, p_sensor_id uuid, p_limit integer default 100)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  if not exists(select 1 from public.fizz_sensors where id=p_sensor_id and customer_id=v_customer and deleted_at is null) then
    return jsonb_build_object('error','not_found');
  end if;
  return jsonb_build_object('readings',coalesce((select jsonb_agg(to_jsonb(x) order by x.observed_at desc,x.received_at desc)
    from (select * from public.fizz_sensor_readings where sensor_id=p_sensor_id
      order by observed_at desc, received_at desc limit least(greatest(coalesce(p_limit,100),1),500)) x), '[]'::jsonb));
end;
$$;

create function public.fizz_tick_simulated(p_token text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_sensor public.fizz_sensors%rowtype; v_tick bigint; v_at timestamptz; v_event text; v_wave double precision;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  v_tick := floor(extract(epoch from now()) / 15)::bigint;
  v_at := to_timestamp(v_tick * 15);
  v_event := 'sample:' || v_tick::text;
  v_wave := sin(v_tick::double precision / 4);
  for v_sensor in select * from public.fizz_sensors where customer_id=v_customer and mode='simulated'
    and status='active' and deleted_at is null and (last_sample_at is null or last_sample_at < v_at) for update skip locked loop
    update public.fizz_sensors set last_sample_at=v_at where id=v_sensor.id;
    case v_sensor.kind
      when 'water' then
        perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'flow_l_min',round((12+v_wave*3)::numeric,1)::float8,null,null,'L/min','simulated');
        perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'leak',null,false,null,null,'simulated');
      when 'gas' then
        perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'concentration_ppm',round((380+v_wave*35)::numeric)::float8,null,null,'ppm','simulated');
        perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'alarm',null,false,null,null,'simulated');
      when 'radio' then perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'rssi_dbm',round((-62+v_wave*8)::numeric)::float8,null,null,'dBm','simulated');
      when 'temperature' then perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'temperature_c',round((22+v_wave*2)::numeric,1)::float8,null,null,'°C','simulated');
      when 'pressure' then perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'pressure_kpa',round((101.3+v_wave*1.2)::numeric,1)::float8,null,null,'kPa','simulated');
      when 'humidity' then perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'humidity_pct',round((48+v_wave*8)::numeric)::float8,null,null,'%','simulated');
      when 'sound' then perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'sound_db',round((42+v_wave*5)::numeric)::float8,null,null,'dB','simulated');
      when 'phone' then
        perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'acceleration_ms2',round((9.8+v_wave*0.5)::numeric,2)::float8,null,null,'m/s²','simulated');
        perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'orientation_deg',round(((v_tick % 360)::double precision)::numeric)::float8,null,null,'°','simulated');
      when 'custom' then perform public.fizz_write_reading(v_sensor.id,v_event,v_at,'value',round((50+v_wave*10)::numeric,1)::float8,null,null,null,'simulated');
    end case;
  end loop;
  return jsonb_build_object('ok',true);
end;
$$;

revoke execute on function public.fizz_session_customer(text), public.fizz_sensor_json(public.fizz_sensors),
  public.fizz_list_sensors(text), public.fizz_create_sensor(text,text,text,text),
  public.fizz_update_sensor(text,uuid,text,text,text,boolean), public.fizz_delete_sensor(text,uuid),
  public.fizz_write_reading(uuid,text,timestamptz,text,double precision,boolean,text,text,text),
  public.fizz_ingest_reading(uuid,text,text,timestamptz,text,double precision,boolean,text,text),
  public.fizz_list_readings(text,uuid,integer), public.fizz_tick_simulated(text)
  from public, anon, authenticated;
grant execute on function public.fizz_session_customer(text), public.fizz_sensor_json(public.fizz_sensors),
  public.fizz_list_sensors(text), public.fizz_create_sensor(text,text,text,text),
  public.fizz_update_sensor(text,uuid,text,text,text,boolean), public.fizz_delete_sensor(text,uuid),
  public.fizz_write_reading(uuid,text,timestamptz,text,double precision,boolean,text,text,text),
  public.fizz_ingest_reading(uuid,text,text,timestamptz,text,double precision,boolean,text,text),
  public.fizz_list_readings(text,uuid,integer), public.fizz_tick_simulated(text)
  to service_role;
