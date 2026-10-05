-- Agent connections. AI agents (Hermes, Claude, and other MCP clients) reach a customer's data
-- through Fizz's MCP server with a revocable bearer token. Devices pair with a short code that an
-- agent or the dashboard claims, and the person at the device approves the pairing there. A paired
-- device becomes an ordinary API-mode sensor, so ingest, alerts, and revocation work unchanged.

create table public.fizz_agent_connections (
  id uuid primary key default gen_random_uuid(),
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  name text not null check (char_length(name) between 1 and 60),
  token_hash bytea not null unique,
  can_create_alerts boolean not null default false,
  created_at timestamptz not null default now(),
  last_used_at timestamptz,
  revoked_at timestamptz
);
create index fizz_agent_connections_customer_idx on public.fizz_agent_connections(customer_id)
  where revoked_at is null;

create table public.fizz_device_pairings (
  id uuid primary key default gen_random_uuid(),
  code_hash bytea not null unique,
  poll_hash bytea not null unique,
  device_name text not null check (char_length(device_name) between 1 and 80),
  platform text check (char_length(platform) <= 80),
  status text not null default 'waiting' check (status in ('waiting', 'claimed', 'approved', 'rejected')),
  customer_id uuid references public.fizz_customers(id) on delete cascade,
  claimed_via text check (claimed_via in ('agent', 'dashboard')),
  agent_id uuid references public.fizz_agent_connections(id) on delete set null,
  agent_name text,
  sensor_id uuid references public.fizz_sensors(id),
  created_at timestamptz not null default now(),
  claimed_at timestamptz,
  finished_at timestamptz,
  expires_at timestamptz not null
);
create index fizz_device_pairings_created_idx on public.fizz_device_pairings(created_at);

alter table public.fizz_agent_connections enable row level security;
alter table public.fizz_device_pairings enable row level security;
revoke all on public.fizz_agent_connections, public.fizz_device_pairings from anon, authenticated;
grant all on public.fizz_agent_connections, public.fizz_device_pairings to service_role;

-- Device metrics reported by the Fizz CLI get units like the built-in sensor metrics.
create or replace function public.fizz_ingest_event(p_sensor_id uuid, p_key text, p_event_id text,
  p_observed_at timestamptz, p_metrics jsonb)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_metric text; v_value jsonb; v_type text; v_result jsonb; v_results jsonb := '[]'::jsonb;
  v_count integer;
  v_numeric double precision; v_boolean boolean; v_text text; v_unit text;
begin
  if p_metrics is null or jsonb_typeof(p_metrics) <> 'object' then
    return jsonb_build_object('error','invalid_reading');
  end if;
  select count(*) into v_count from jsonb_object_keys(p_metrics);
  if v_count not between 1 and 16 then return jsonb_build_object('error','invalid_reading'); end if;
  begin
    for v_metric, v_value in select key,value from jsonb_each(p_metrics) loop
      v_type := jsonb_typeof(v_value);
      if v_metric !~ '^[a-z][a-z0-9_]{0,39}$'
        or v_type not in ('number','boolean','string') then
        raise exception 'invalid_reading' using errcode='22023';
      end if;
      v_numeric := null; v_boolean := null; v_text := null;
      if v_type = 'number' then v_numeric := (v_value #>> '{}')::double precision;
      elsif v_type = 'boolean' then v_boolean := (v_value #>> '{}')::boolean;
      else v_text := v_value #>> '{}'; end if;
      v_unit := case v_metric
        when 'flow_l_min' then 'L/min' when 'concentration_ppm' then 'ppm'
        when 'rssi_dbm' then 'dBm' when 'temperature_c' then '°C'
        when 'pressure_kpa' then 'kPa' when 'humidity_pct' then '%'
        when 'sound_db' then 'dB' when 'acceleration_ms2' then 'm/s²'
        when 'orientation_deg' then '°'
        when 'cpu_temperature_c' then '°C'
        when 'memory_used_pct' then '%' when 'disk_used_pct' then '%'
        else null end;
      v_result := public.fizz_ingest_reading(p_sensor_id,p_key,p_event_id,p_observed_at,
        v_metric,v_numeric,v_boolean,v_text,v_unit);
      if v_result ? 'error' then
        raise exception '%', v_result->>'error' using errcode='22023';
      end if;
      v_results := v_results || jsonb_build_array(v_result);
    end loop;
  exception when sqlstate '22023' then
    return jsonb_build_object('error',SQLERRM);
  end;
  return jsonb_build_object('accepted',v_results);
end;
$$;

-- Sample streams advance for agent reads too, so the per-customer tick is split out and the
-- session-token version delegates to it.
create function public.fizz_tick_simulated_for(p_customer uuid)
returns void language plpgsql security definer set search_path = '' as $$
declare v_sensor public.fizz_sensors%rowtype; v_tick bigint; v_at timestamptz; v_event text; v_wave double precision;
begin
  v_tick := floor(extract(epoch from now()) / 15)::bigint;
  v_at := to_timestamp(v_tick * 15);
  v_event := 'sample:' || v_tick::text;
  v_wave := sin(v_tick::double precision / 4);
  for v_sensor in select * from public.fizz_sensors where customer_id=p_customer and mode='simulated'
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
end;
$$;

create or replace function public.fizz_tick_simulated(p_token text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  perform public.fizz_tick_simulated_for(v_customer);
  return jsonb_build_object('ok',true);
end;
$$;

-- ---------------------------------------------------------------------------------------------
-- Agent tokens, managed from the signed-in dashboard. The plaintext token is returned once.

create function public.fizz_agent_create(p_token text, p_name text, p_can_create_alerts boolean)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_name text := trim(coalesce(p_name, '')); v_raw text; v_row public.fizz_agent_connections%rowtype;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  if char_length(v_name) not between 1 and 60 then return jsonb_build_object('error','invalid_name'); end if;
  if (select count(*) from public.fizz_agent_connections where customer_id = v_customer and revoked_at is null) >= 20 then
    return jsonb_build_object('error','too_many');
  end if;
  v_raw := 'fizz_agent_' || encode(extensions.gen_random_bytes(32), 'hex');
  insert into public.fizz_agent_connections(customer_id, name, token_hash, can_create_alerts)
  values (v_customer, v_name, extensions.digest(v_raw, 'sha256'), coalesce(p_can_create_alerts, false))
  returning * into v_row;
  return jsonb_build_object('token', v_raw, 'connection', jsonb_build_object(
    'id', v_row.id, 'name', v_row.name, 'can_create_alerts', v_row.can_create_alerts,
    'created_at', v_row.created_at, 'last_used_at', v_row.last_used_at));
end;
$$;

create function public.fizz_agent_list(p_token text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  return jsonb_build_object('connections', coalesce((
    select jsonb_agg(jsonb_build_object('id', a.id, 'name', a.name, 'can_create_alerts', a.can_create_alerts,
      'created_at', a.created_at, 'last_used_at', a.last_used_at) order by a.created_at desc)
    from public.fizz_agent_connections a where a.customer_id = v_customer and a.revoked_at is null), '[]'::jsonb));
end;
$$;

create function public.fizz_agent_revoke(p_token text, p_id uuid)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  update public.fizz_agent_connections set revoked_at = now()
  where id = p_id and customer_id = v_customer and revoked_at is null;
  if not found then return jsonb_build_object('error','not_found'); end if;
  return jsonb_build_object('ok', true);
end;
$$;

-- ---------------------------------------------------------------------------------------------
-- Device pairing. Codes look like 7K3P-Q9DM (Crockford base32, 40 random bits), expire after
-- 15 minutes, are rate-limited, and only become a connection after approval on the device.

create function public.fizz_normalize_pairing_code(p_code text)
returns text language plpgsql immutable set search_path = '' as $$
declare v_code text := regexp_replace(upper(coalesce(p_code, '')), '[^A-Z0-9]', '', 'g');
begin
  if length(v_code) = 12 and left(v_code, 4) = 'FIZZ' then v_code := substr(v_code, 5); end if;
  v_code := translate(v_code, 'OIL', '011');
  if v_code !~ '^[0-9A-HJKMNP-TV-Z]{8}$' then return null; end if;
  return v_code;
end;
$$;

create function public.fizz_pair_start(p_name text, p_platform text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_alphabet constant text := '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
  v_name text := left(trim(coalesce(p_name, '')), 80);
  v_platform text := nullif(left(trim(coalesce(p_platform, '')), 80), '');
  v_bytes bytea; v_code text; v_secret text; v_expires timestamptz := now() + interval '15 minutes';
begin
  if not public.fizz_rate_limit_hit('pair_start_ip', p_ip, 10, interval '1 hour') then
    return jsonb_build_object('error','rate_limited');
  end if;
  if v_name = '' then v_name := 'My device'; end if;
  if random() < 0.05 then
    delete from public.fizz_device_pairings where created_at < now() - interval '1 day' and status <> 'approved';
  end if;
  v_secret := encode(extensions.gen_random_bytes(32), 'hex');
  for attempt in 1..5 loop
    v_bytes := extensions.gen_random_bytes(8);
    v_code := '';
    for j in 0..7 loop
      v_code := v_code || substr(v_alphabet, get_byte(v_bytes, j) % 32 + 1, 1);
    end loop;
    begin
      insert into public.fizz_device_pairings(code_hash, poll_hash, device_name, platform, expires_at)
      values (extensions.digest(v_code, 'sha256'), extensions.digest(v_secret, 'sha256'), v_name, v_platform, v_expires);
      return jsonb_build_object('code', substr(v_code, 1, 4) || '-' || substr(v_code, 5, 4),
        'poll_secret', v_secret, 'expires_at', v_expires);
    exception when unique_violation then
      -- A code collision is astronomically rare; draw again.
    end;
  end loop;
  return jsonb_build_object('error','unavailable');
end;
$$;

-- The device polls with its secret to see whether someone claimed the code.
create function public.fizz_pair_status(p_poll_secret text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_pair public.fizz_device_pairings%rowtype; v_username text;
begin
  if p_poll_secret !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error','not_found'); end if;
  select * into v_pair from public.fizz_device_pairings where poll_hash = extensions.digest(p_poll_secret, 'sha256');
  if not found then return jsonb_build_object('error','not_found'); end if;
  if v_pair.status in ('waiting', 'claimed') and v_pair.expires_at <= now() then
    return jsonb_build_object('status','expired');
  end if;
  select username into v_username from public.fizz_customers where id = v_pair.customer_id;
  return jsonb_build_object('status', v_pair.status, 'device_name', v_pair.device_name,
    'expires_at', v_pair.expires_at, 'account', v_username,
    'claimed_via', v_pair.claimed_via, 'agent_name', v_pair.agent_name);
end;
$$;

-- Approval happens on the device. Approving creates an API-mode sensor and returns its key once.
create function public.fizz_pair_decide(p_poll_secret text, p_approve boolean)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_pair public.fizz_device_pairings%rowtype; v_key text; v_sensor public.fizz_sensors%rowtype; v_username text;
begin
  if p_poll_secret !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error','not_found'); end if;
  select * into v_pair from public.fizz_device_pairings
  where poll_hash = extensions.digest(p_poll_secret, 'sha256') for update;
  if not found then return jsonb_build_object('error','not_found'); end if;
  if v_pair.expires_at <= now() and v_pair.status in ('waiting', 'claimed') then
    return jsonb_build_object('error','expired');
  end if;
  if v_pair.status <> 'claimed' then return jsonb_build_object('error','not_claimed'); end if;
  if not coalesce(p_approve, false) then
    update public.fizz_device_pairings set status = 'rejected', finished_at = now() where id = v_pair.id;
    return jsonb_build_object('status','rejected');
  end if;
  v_key := 'fizz_' || encode(extensions.gen_random_bytes(32), 'hex');
  insert into public.fizz_sensors(customer_id, kind, name, mode, api_key_hash)
  values (v_pair.customer_id, 'custom', v_pair.device_name, 'api', extensions.digest(v_key, 'sha256'))
  returning * into v_sensor;
  update public.fizz_device_pairings set status = 'approved', sensor_id = v_sensor.id, finished_at = now()
  where id = v_pair.id;
  select username into v_username from public.fizz_customers where id = v_pair.customer_id;
  return jsonb_build_object('status','approved', 'sensor_id', v_sensor.id, 'api_key', v_key,
    'account', v_username, 'device_name', v_sensor.name);
end;
$$;

-- Shared by agents and the dashboard: attach a waiting code to a customer.
create function public.fizz_pair_claim_for(p_customer uuid, p_code text, p_via text,
  p_agent_id uuid, p_agent_name text, p_device_name text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_code text := public.fizz_normalize_pairing_code(p_code); v_pair public.fizz_device_pairings%rowtype;
  v_name text := nullif(left(trim(coalesce(p_device_name, '')), 80), '');
begin
  if not public.fizz_rate_limit_hit('pair_claim', p_customer::text, 20, interval '1 hour') then
    return jsonb_build_object('error','rate_limited');
  end if;
  if v_code is null then return jsonb_build_object('error','invalid_code'); end if;
  select * into v_pair from public.fizz_device_pairings
  where code_hash = extensions.digest(v_code, 'sha256') for update;
  if not found or v_pair.expires_at <= now() or v_pair.status in ('approved', 'rejected') then
    return jsonb_build_object('error','invalid_code');
  end if;
  if v_pair.status = 'claimed' then
    if v_pair.customer_id = p_customer then
      return jsonb_build_object('device_name', v_pair.device_name, 'platform', v_pair.platform,
        'status', 'waiting_for_approval', 'expires_at', v_pair.expires_at);
    end if;
    return jsonb_build_object('error','invalid_code');
  end if;
  update public.fizz_device_pairings
  set status = 'claimed', customer_id = p_customer, claimed_via = p_via, agent_id = p_agent_id,
      agent_name = p_agent_name, claimed_at = now(), device_name = coalesce(v_name, device_name)
  where id = v_pair.id returning * into v_pair;
  return jsonb_build_object('device_name', v_pair.device_name, 'platform', v_pair.platform,
    'status', 'waiting_for_approval', 'expires_at', v_pair.expires_at);
end;
$$;

create function public.fizz_pair_claim(p_token text, p_code text, p_device_name text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_token);
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  return public.fizz_pair_claim_for(v_customer, p_code, 'dashboard', null, null, p_device_name);
end;
$$;

-- A device can remove itself with its own key (fizz unpair). Alerts are disabled by trigger.
create function public.fizz_device_unpair(p_sensor_id uuid, p_key text)
returns jsonb language plpgsql security definer set search_path = '' as $$
begin
  if p_key !~ '^fizz_[0-9a-f]{64}$' then return jsonb_build_object('error','unauthorized'); end if;
  update public.fizz_sensors set deleted_at = now(), status = 'paused', api_key_hash = null
  where id = p_sensor_id and mode = 'api' and deleted_at is null
    and api_key_hash = extensions.digest(p_key, 'sha256');
  if not found then return jsonb_build_object('error','unauthorized'); end if;
  return jsonb_build_object('ok', true);
end;
$$;

-- ---------------------------------------------------------------------------------------------
-- MCP tool calls. One call authenticates the agent token, applies its rate limit and scope, and
-- runs the tool, so each tool call costs a single round trip.

create function public.fizz_agent_reading_json(p_reading public.fizz_sensor_readings)
returns jsonb language sql stable set search_path = '' as $$
  select jsonb_build_object('metric', p_reading.metric,
    'value', coalesce(to_jsonb(p_reading.numeric_value), to_jsonb(p_reading.boolean_value), to_jsonb(p_reading.text_value)),
    'unit', p_reading.unit, 'observed_at', p_reading.observed_at, 'source', p_reading.source);
$$;

create function public.fizz_agent_call(p_agent_token text, p_ip text, p_tool text, p_args jsonb)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_conn public.fizz_agent_connections%rowtype;
  v_args jsonb := coalesce(p_args, '{}'::jsonb);
  v_sensor public.fizz_sensors%rowtype;
  v_limit integer; v_metric text; v_comparator text; v_threshold double precision; v_unit text;
  v_rule public.fizz_alert_rules%rowtype;
begin
  if p_agent_token is null or p_agent_token !~ '^fizz_agent_[0-9a-f]{64}$' then
    return jsonb_build_object('error','unauthorized');
  end if;
  select a.* into v_conn from public.fizz_agent_connections a
  where a.token_hash = extensions.digest(p_agent_token, 'sha256') and a.revoked_at is null;
  if not found then return jsonb_build_object('error','unauthorized'); end if;
  if not public.fizz_rate_limit_hit('agent', v_conn.id::text, 120, interval '1 minute') then
    return jsonb_build_object('error','rate_limited');
  end if;
  update public.fizz_agent_connections set last_used_at = now()
  where id = v_conn.id and (last_used_at is null or last_used_at < now() - interval '1 minute');
  if jsonb_typeof(v_args) <> 'object' then
    return jsonb_build_object('error','invalid_arguments','message','Arguments must be an object.');
  end if;

  if p_tool = 'connection' then
    return jsonb_build_object('result', jsonb_build_object('name', v_conn.name, 'can_create_alerts', v_conn.can_create_alerts));

  elsif p_tool = 'list_sensors' then
    perform public.fizz_tick_simulated_for(v_conn.customer_id);
    return jsonb_build_object('result', jsonb_build_object('sensors', coalesce((
      select jsonb_agg(jsonb_build_object('id', s.id, 'name', s.name, 'type', s.kind, 'mode', s.mode,
        'status', s.status,
        'latest', (select public.fizz_agent_reading_json(r) from public.fizz_sensor_readings r
          where r.sensor_id = s.id order by r.observed_at desc, r.received_at desc limit 1))
        order by s.created_at, s.id)
      from public.fizz_sensors s where s.customer_id = v_conn.customer_id and s.deleted_at is null), '[]'::jsonb)));

  elsif p_tool = 'get_readings' then
    if coalesce(v_args->>'sensor_id', '') !~ '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$' then
      return jsonb_build_object('error','invalid_arguments','message','sensor_id must be a sensor ID from list_sensors.');
    end if;
    select * into v_sensor from public.fizz_sensors
    where id = (v_args->>'sensor_id')::uuid and customer_id = v_conn.customer_id and deleted_at is null;
    if not found then
      return jsonb_build_object('error','not_found','message','No sensor with that ID. Call list_sensors for valid IDs.');
    end if;
    v_limit := case when jsonb_typeof(v_args->'limit') = 'number' then least(greatest((v_args->>'limit')::numeric::integer, 1), 100) else 20 end;
    v_metric := nullif(v_args->>'metric', '');
    perform public.fizz_tick_simulated_for(v_conn.customer_id);
    return jsonb_build_object('result', jsonb_build_object(
      'sensor', jsonb_build_object('id', v_sensor.id, 'name', v_sensor.name, 'type', v_sensor.kind),
      'readings', coalesce((select jsonb_agg(public.fizz_agent_reading_json(x) order by x.observed_at desc, x.received_at desc)
        from (select * from public.fizz_sensor_readings where sensor_id = v_sensor.id
          and (v_metric is null or metric = v_metric)
          order by observed_at desc, received_at desc limit v_limit) x), '[]'::jsonb)));

  elsif p_tool = 'list_alerts' then
    return jsonb_build_object('result', jsonb_build_object(
      'rules', coalesce((select jsonb_agg(jsonb_build_object('id', r.id, 'sensor_id', r.sensor_id, 'sensor_name', s.name,
          'metric', r.metric, 'comparator', r.comparator, 'threshold', r.threshold, 'unit', r.unit,
          'enabled', r.enabled, 'triggered', not r.armed, 'calls_phone', r.notify_channel = 'call')
        order by r.created_at desc)
        from public.fizz_alert_rules r join public.fizz_sensors s on s.id = r.sensor_id
        where r.customer_id = v_conn.customer_id and s.deleted_at is null), '[]'::jsonb),
      'recent_triggers', coalesce((select jsonb_agg(jsonb_build_object('sensor_id', e.sensor_id, 'sensor_name', s.name,
          'metric', e.metric, 'value', e.numeric_value, 'comparator', e.comparator, 'threshold', e.threshold,
          'unit', e.unit, 'observed_at', e.observed_at) order by e.created_at desc)
        from (select * from public.fizz_alert_events where customer_id = v_conn.customer_id
          order by created_at desc limit 20) e join public.fizz_sensors s on s.id = e.sensor_id), '[]'::jsonb)));

  elsif p_tool = 'create_alert' then
    if not v_conn.can_create_alerts then
      return jsonb_build_object('error','forbidden','message','This agent connection is read-only. The account owner can allow alert creation when connecting an agent.');
    end if;
    if coalesce(v_args->>'sensor_id', '') !~ '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$' then
      return jsonb_build_object('error','invalid_arguments','message','sensor_id must be a sensor ID from list_sensors.');
    end if;
    select * into v_sensor from public.fizz_sensors
    where id = (v_args->>'sensor_id')::uuid and customer_id = v_conn.customer_id and deleted_at is null;
    if not found then
      return jsonb_build_object('error','not_found','message','No sensor with that ID. Call list_sensors for valid IDs.');
    end if;
    v_metric := v_args->>'metric';
    v_comparator := v_args->>'comparator';
    if v_metric is null or v_metric !~ '^[a-z][a-z0-9_]{0,63}$' or v_comparator is null
      or v_comparator not in ('gt','gte','lt','lte') or jsonb_typeof(v_args->'threshold') <> 'number' then
      return jsonb_build_object('error','invalid_arguments','message','Provide metric, comparator (gt, gte, lt, lte), and a numeric threshold.');
    end if;
    v_threshold := (v_args->>'threshold')::double precision;
    -- Rules target a metric the sensor has reported; the unit defaults to that reading's unit.
    select coalesce(unit, '') into v_unit from public.fizz_sensor_readings
    where sensor_id = v_sensor.id and metric = v_metric and numeric_value is not null
    order by received_at desc limit 1;
    if not found then
      return jsonb_build_object('error','invalid_arguments','message','That sensor has not reported a numeric value for this metric yet. Use get_readings to see its metrics.');
    end if;
    if v_args ? 'unit' and coalesce(v_args->>'unit', '') <> v_unit then
      return jsonb_build_object('error','invalid_arguments','message', format('This metric is reported in %s; use that unit or omit it.', coalesce(nullif(v_unit, ''), 'no unit')));
    end if;
    perform pg_advisory_xact_lock(hashtextextended(v_conn.customer_id::text, 0));
    select r.* into v_rule from public.fizz_alert_rules r
    where r.customer_id = v_conn.customer_id and r.enabled and r.sensor_id = v_sensor.id and r.metric = v_metric
      and r.comparator = v_comparator and r.threshold = v_threshold and r.unit = v_unit and r.notify_channel = 'none'
    limit 1;
    if found then
      return jsonb_build_object('result', jsonb_build_object('created', false, 'rule', jsonb_build_object('id', v_rule.id,
        'sensor_id', v_rule.sensor_id, 'metric', v_rule.metric, 'comparator', v_rule.comparator,
        'threshold', v_rule.threshold, 'unit', v_rule.unit)));
    end if;
    if (select count(*) from public.fizz_alert_rules where customer_id = v_conn.customer_id and enabled) >= 100 then
      return jsonb_build_object('error','invalid_arguments','message','This account already has 100 active alerts. Pause or delete some first.');
    end if;
    insert into public.fizz_alert_rules(customer_id, sensor_id, metric, comparator, threshold, unit)
    values (v_conn.customer_id, v_sensor.id, v_metric, v_comparator, v_threshold, v_unit) returning * into v_rule;
    return jsonb_build_object('result', jsonb_build_object('created', true, 'rule', jsonb_build_object('id', v_rule.id,
      'sensor_id', v_rule.sensor_id, 'metric', v_rule.metric, 'comparator', v_rule.comparator,
      'threshold', v_rule.threshold, 'unit', v_rule.unit)));

  elsif p_tool = 'pair_device' then
    return (select case when x ? 'error' then x || jsonb_build_object('message', case x->>'error'
        when 'rate_limited' then 'Too many pairing attempts. Try again in an hour.'
        else 'That code is not valid or has expired. Run fizz pair on the device for a new code.' end)
      else jsonb_build_object('result', x || jsonb_build_object('next_step',
        'Ask the person at the device to approve the pairing in the Fizz CLI. It becomes a sensor once approved.')) end
      from public.fizz_pair_claim_for(v_conn.customer_id, v_args->>'code', 'agent', v_conn.id, v_conn.name,
        v_args->>'name') x);
  end if;
  return jsonb_build_object('error','unknown_tool');
end;
$$;

revoke execute on function public.fizz_tick_simulated_for(uuid), public.fizz_agent_create(text, text, boolean),
  public.fizz_agent_list(text), public.fizz_agent_revoke(text, uuid), public.fizz_normalize_pairing_code(text),
  public.fizz_pair_start(text, text, text), public.fizz_pair_status(text), public.fizz_pair_decide(text, boolean),
  public.fizz_pair_claim_for(uuid, text, text, uuid, text, text), public.fizz_pair_claim(text, text, text),
  public.fizz_device_unpair(uuid, text), public.fizz_agent_reading_json(public.fizz_sensor_readings),
  public.fizz_agent_call(text, text, text, jsonb)
  from public, anon, authenticated;
grant execute on function public.fizz_agent_create(text, text, boolean), public.fizz_agent_list(text),
  public.fizz_agent_revoke(text, uuid), public.fizz_pair_start(text, text, text), public.fizz_pair_status(text),
  public.fizz_pair_decide(text, boolean), public.fizz_pair_claim(text, text, text),
  public.fizz_device_unpair(uuid, text), public.fizz_agent_call(text, text, text, jsonb)
  to service_role;
