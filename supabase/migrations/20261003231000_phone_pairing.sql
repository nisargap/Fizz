create table public.fizz_phone_pairings (
  token_hash bytea primary key,
  sensor_id uuid not null references public.fizz_sensors(id) on delete cascade,
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  expires_at timestamptz not null,
  claimed_at timestamptz,
  created_at timestamptz not null default now()
);
create index fizz_phone_pairings_sensor_idx on public.fizz_phone_pairings(sensor_id);

create table public.fizz_phone_devices (
  token_hash bytea primary key,
  sensor_id uuid not null references public.fizz_sensors(id) on delete cascade,
  expires_at timestamptz not null,
  revoked_at timestamptz,
  last_seen_at timestamptz,
  created_at timestamptz not null default now()
);
create index fizz_phone_devices_sensor_idx on public.fizz_phone_devices(sensor_id);

create table public.fizz_phone_clips (
  id uuid primary key default gen_random_uuid(),
  sensor_id uuid not null references public.fizz_sensors(id) on delete cascade,
  device_hash bytea not null references public.fizz_phone_devices(token_hash) on delete cascade,
  mime_type text not null,
  audio bytea not null,
  created_at timestamptz not null default now(),
  constraint fizz_phone_clip_size check (octet_length(audio) <= 262144)
);
create index fizz_phone_clips_sensor_idx on public.fizz_phone_clips(sensor_id, created_at desc);

alter table public.fizz_phone_pairings enable row level security;
alter table public.fizz_phone_devices enable row level security;
alter table public.fizz_phone_clips enable row level security;
revoke all on public.fizz_phone_pairings, public.fizz_phone_devices, public.fizz_phone_clips from anon, authenticated;
grant all on public.fizz_phone_pairings, public.fizz_phone_devices, public.fizz_phone_clips to service_role;

create or replace function public.fizz_phone_create_pairing(p_owner_token text, p_sensor_id uuid)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_token bytea;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_owner_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_owner_token, 'hex'), 'sha256')
    and expires_at > now();
  if v_customer is null then return jsonb_build_object('error', 'unauthorized'); end if;
  if not exists (select 1 from public.fizz_sensors where id = p_sensor_id and customer_id = v_customer and kind = 'phone' and deleted_at is null) then
    return jsonb_build_object('error', 'sensor_not_found');
  end if;
  update public.fizz_sensors set mode = 'phone', status = 'active', api_key_hash = null, last_sample_at = null
  where id = p_sensor_id;
  delete from public.fizz_phone_pairings where sensor_id = p_sensor_id and claimed_at is null;
  v_token := extensions.gen_random_bytes(32);
  insert into public.fizz_phone_pairings(token_hash, sensor_id, customer_id, expires_at)
  values (extensions.digest(v_token, 'sha256'), p_sensor_id, v_customer, now() + interval '15 minutes');
  return jsonb_build_object('token', encode(v_token, 'hex'), 'expires_in_seconds', 900);
end;
$$;

create or replace function public.fizz_phone_claim(p_pairing_token text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_pairing public.fizz_phone_pairings%rowtype; v_token bytea;
begin
  if p_pairing_token !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error', 'invalid_link'); end if;
  select * into v_pairing from public.fizz_phone_pairings
  where token_hash = extensions.digest(decode(p_pairing_token, 'hex'), 'sha256') for update;
  if not found or v_pairing.claimed_at is not null or v_pairing.expires_at <= now()
     or not exists (select 1 from public.fizz_sensors where id = v_pairing.sensor_id and deleted_at is null and kind = 'phone') then
    return jsonb_build_object('error', 'invalid_link');
  end if;
  update public.fizz_phone_pairings set claimed_at = now() where token_hash = v_pairing.token_hash;
  v_token := extensions.gen_random_bytes(32);
  insert into public.fizz_phone_devices(token_hash, sensor_id, expires_at)
  values (extensions.digest(v_token, 'sha256'), v_pairing.sensor_id, now() + interval '30 days');
  return jsonb_build_object('device_token', encode(v_token, 'hex'), 'sensor_id', v_pairing.sensor_id, 'expires_in_seconds', 2592000);
end;
$$;

create or replace function public.fizz_phone_status(p_owner_token text, p_sensor_id uuid)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_owner_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_owner_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error', 'unauthorized'); end if;
  if not exists (select 1 from public.fizz_sensors where id = p_sensor_id and customer_id = v_customer and kind = 'phone' and deleted_at is null) then
    return jsonb_build_object('error', 'sensor_not_found');
  end if;
  return jsonb_build_object('connected', exists(
    select 1 from public.fizz_phone_devices where sensor_id = p_sensor_id and revoked_at is null and expires_at > now()
  ), 'device_count', (select count(*) from public.fizz_phone_devices where sensor_id = p_sensor_id and revoked_at is null and expires_at > now()),
    'last_seen_at', (select max(last_seen_at) from public.fizz_phone_devices where sensor_id = p_sensor_id and revoked_at is null));
end;
$$;

create or replace function public.fizz_phone_revoke(p_owner_token text, p_sensor_id uuid)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_owner_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_owner_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error', 'unauthorized'); end if;
  if not exists (select 1 from public.fizz_sensors where id = p_sensor_id and customer_id = v_customer and kind = 'phone' and deleted_at is null) then
    return jsonb_build_object('error', 'sensor_not_found');
  end if;
  update public.fizz_phone_devices set revoked_at = now() where sensor_id = p_sensor_id and revoked_at is null;
  delete from public.fizz_phone_pairings where sensor_id = p_sensor_id and claimed_at is null;
  return jsonb_build_object('ok', true);
end;
$$;

create or replace function public.fizz_phone_reading(p_device_token text, p_event_id text, p_metric text,
  p_numeric_value double precision, p_unit text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_device public.fizz_phone_devices%rowtype;
begin
  if p_device_token !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error', 'unauthorized'); end if;
  select * into v_device from public.fizz_phone_devices
  where token_hash = extensions.digest(decode(p_device_token, 'hex'), 'sha256')
    and revoked_at is null and expires_at > now() for update;
  if not found or not exists (select 1 from public.fizz_sensors where id = v_device.sensor_id and deleted_at is null and kind = 'phone') then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  if p_metric not in ('acceleration_x', 'acceleration_y', 'acceleration_z', 'rotation_alpha', 'rotation_beta', 'rotation_gamma', 'latitude', 'longitude', 'location_accuracy', 'voice_level')
     or p_numeric_value is null or p_numeric_value = 'NaN'::float8 or abs(p_numeric_value) > 100000
     or length(p_event_id) not between 1 and 100 then
    return jsonb_build_object('error', 'invalid_reading');
  end if;
  insert into public.fizz_sensor_readings(sensor_id, event_id, observed_at, metric, numeric_value, unit, source)
  values (v_device.sensor_id, p_event_id, now(), p_metric, p_numeric_value, p_unit, 'phone')
  on conflict (sensor_id, event_id, metric) do nothing;
  update public.fizz_phone_devices set last_seen_at = now() where token_hash = v_device.token_hash;
  return jsonb_build_object('ok', true, 'sensor_id', v_device.sensor_id);
end;
$$;

create or replace function public.fizz_phone_disconnect(p_device_token text)
returns jsonb language plpgsql security definer set search_path = '' as $$
begin
  if p_device_token !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error', 'unauthorized'); end if;
  update public.fizz_phone_devices set revoked_at = now()
  where token_hash = extensions.digest(decode(p_device_token, 'hex'), 'sha256') and revoked_at is null;
  return jsonb_build_object('ok', true);
end;
$$;

create or replace function public.fizz_phone_transcript(p_device_token text, p_event_id text, p_text text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_device public.fizz_phone_devices%rowtype;
begin
  if p_device_token !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error', 'unauthorized'); end if;
  select * into v_device from public.fizz_phone_devices
  where token_hash = extensions.digest(decode(p_device_token, 'hex'), 'sha256')
    and revoked_at is null and expires_at > now() for update;
  if not found or not exists (select 1 from public.fizz_sensors where id = v_device.sensor_id and deleted_at is null and kind = 'phone') then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  if length(p_event_id) not between 1 and 100 or length(trim(p_text)) not between 1 and 500 then
    return jsonb_build_object('error', 'invalid_transcript');
  end if;
  insert into public.fizz_sensor_readings(sensor_id,event_id,observed_at,metric,text_value,source)
  values (v_device.sensor_id,p_event_id,now(),'voice_transcript',trim(p_text),'phone')
  on conflict (sensor_id,event_id,metric) do nothing;
  update public.fizz_phone_devices set last_seen_at = now() where token_hash = v_device.token_hash;
  return jsonb_build_object('ok', true, 'sensor_id', v_device.sensor_id);
end;
$$;

create or replace function public.fizz_phone_clip(p_device_token text, p_mime_type text, p_audio_base64 text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_device public.fizz_phone_devices%rowtype; v_audio bytea; v_clip_id uuid;
begin
  if p_device_token !~ '^[0-9a-f]{64}$' then return jsonb_build_object('error', 'unauthorized'); end if;
  select * into v_device from public.fizz_phone_devices
  where token_hash = extensions.digest(decode(p_device_token, 'hex'), 'sha256')
    and revoked_at is null and expires_at > now() for update;
  if not found or not exists (select 1 from public.fizz_sensors where id = v_device.sensor_id and deleted_at is null and kind = 'phone') then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  if p_mime_type not in ('audio/webm', 'audio/webm;codecs=opus', 'audio/mp4', 'audio/ogg;codecs=opus')
     or length(p_audio_base64) > 349528 then return jsonb_build_object('error', 'invalid_clip'); end if;
  begin v_audio := decode(p_audio_base64, 'base64');
  exception when others then return jsonb_build_object('error', 'invalid_clip'); end;
  if octet_length(v_audio) < 100 or octet_length(v_audio) > 262144 then
    return jsonb_build_object('error', 'invalid_clip');
  end if;
  insert into public.fizz_phone_clips(sensor_id, device_hash, mime_type, audio)
  values (v_device.sensor_id, v_device.token_hash, p_mime_type, v_audio) returning id into v_clip_id;
  insert into public.fizz_sensor_readings(sensor_id, event_id, observed_at, metric, numeric_value, unit, source)
  values (v_device.sensor_id, v_clip_id::text, now(), 'voice_clip', octet_length(v_audio), 'bytes', 'phone');
  update public.fizz_phone_devices set last_seen_at = now() where token_hash = v_device.token_hash;
  return jsonb_build_object('ok', true, 'clip_id', v_clip_id, 'size_bytes', octet_length(v_audio));
end;
$$;

create or replace function public.fizz_phone_get_clip(p_owner_token text, p_clip_id uuid)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid; v_clip public.fizz_phone_clips%rowtype;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_owner_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_owner_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error', 'unauthorized'); end if;
  select c.* into v_clip from public.fizz_phone_clips c
  join public.fizz_sensors s on s.id = c.sensor_id
  where c.id = p_clip_id and s.customer_id = v_customer and s.deleted_at is null;
  if not found then return jsonb_build_object('error', 'clip_not_found'); end if;
  return jsonb_build_object('clip_id', v_clip.id, 'sensor_id', v_clip.sensor_id,
    'mime_type', v_clip.mime_type, 'audio_base64', encode(v_clip.audio, 'base64'), 'created_at', v_clip.created_at);
end;
$$;

revoke execute on function public.fizz_phone_create_pairing(text,uuid), public.fizz_phone_claim(text),
  public.fizz_phone_status(text,uuid), public.fizz_phone_revoke(text,uuid),
  public.fizz_phone_reading(text,text,text,double precision,text), public.fizz_phone_clip(text,text,text),
  public.fizz_phone_disconnect(text), public.fizz_phone_get_clip(text,uuid),
  public.fizz_phone_transcript(text,text,text)
  from public, anon, authenticated;
grant execute on function public.fizz_phone_create_pairing(text,uuid), public.fizz_phone_claim(text),
  public.fizz_phone_status(text,uuid), public.fizz_phone_revoke(text,uuid),
  public.fizz_phone_reading(text,text,text,double precision,text), public.fizz_phone_clip(text,text,text),
  public.fizz_phone_disconnect(text), public.fizz_phone_get_clip(text,uuid),
  public.fizz_phone_transcript(text,text,text)
  to service_role;
