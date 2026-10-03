create table public.fizz_sensor_choices (
  id uuid primary key default gen_random_uuid(),
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  kind text not null check (kind in ('water', 'gas', 'radio', 'temperature', 'pressure', 'humidity', 'sound', 'phone', 'custom')),
  created_at timestamptz not null default now(),
  unique (customer_id, kind)
);

create index fizz_sensor_choices_customer_id_idx on public.fizz_sensor_choices(customer_id);
alter table public.fizz_sensor_choices enable row level security;
revoke all on public.fizz_sensor_choices from anon, authenticated;
grant all on public.fizz_sensor_choices to service_role;

create function public.fizz_get_sensor_choices(p_token text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare
  v_customer_id uuid;
  v_choices jsonb;
begin
  if p_token is null or p_token !~ '^[0-9a-f]{64}$' then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  select customer_id into v_customer_id
  from public.fizz_sessions
  where token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256')
    and expires_at > now();
  if v_customer_id is null then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  select coalesce(jsonb_agg(kind order by created_at, kind), '[]'::jsonb)
  into v_choices from public.fizz_sensor_choices where customer_id = v_customer_id;
  return jsonb_build_object('choices', v_choices);
end;
$$;

create function public.fizz_add_sensor_choice(p_token text, p_kind text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_customer_id uuid;
begin
  if p_token is null or p_token !~ '^[0-9a-f]{64}$' then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  if p_kind not in ('water', 'gas', 'radio', 'temperature', 'pressure', 'humidity', 'sound', 'phone', 'custom') then
    return jsonb_build_object('error', 'invalid_kind');
  end if;
  select customer_id into v_customer_id
  from public.fizz_sessions
  where token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256')
    and expires_at > now();
  if v_customer_id is null then
    return jsonb_build_object('error', 'unauthorized');
  end if;
  insert into public.fizz_sensor_choices(customer_id, kind)
  values (v_customer_id, p_kind)
  on conflict (customer_id, kind) do nothing;
  return public.fizz_get_sensor_choices(p_token);
end;
$$;

revoke execute on function public.fizz_get_sensor_choices(text),
  public.fizz_add_sensor_choice(text, text) from public, anon, authenticated;
grant execute on function public.fizz_get_sensor_choices(text),
  public.fizz_add_sensor_choice(text, text) to service_role;
