create extension if not exists pgcrypto with schema extensions;

create table if not exists public.fizz_customers (
  id uuid primary key default gen_random_uuid(),
  username text not null unique check (username ~ '^[a-z][a-z0-9_]{2,23}$'),
  code_hash text not null,
  failed_attempts integer not null default 0,
  locked_until timestamptz,
  created_at timestamptz not null default now()
);

create table if not exists public.fizz_sessions (
  token_hash bytea primary key,
  customer_id uuid not null references public.fizz_customers(id) on delete cascade,
  expires_at timestamptz not null,
  created_at timestamptz not null default now()
);
create index if not exists fizz_sessions_customer_id_idx on public.fizz_sessions(customer_id);
create index if not exists fizz_sessions_expires_at_idx on public.fizz_sessions(expires_at);

alter table public.fizz_customers enable row level security;
alter table public.fizz_sessions enable row level security;
revoke all on public.fizz_customers, public.fizz_sessions from anon, authenticated;
grant all on public.fizz_customers, public.fizz_sessions to service_role;

create or replace function public.fizz_register(p_username text, p_code text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_customer public.fizz_customers%rowtype;
  v_token bytea;
begin
  p_username := lower(trim(p_username));
  if p_username !~ '^[a-z][a-z0-9_]{2,23}$' or p_code !~ '^[0-9]{6}$' then
    raise exception 'Invalid username or code' using errcode = '22023';
  end if;
  insert into public.fizz_customers(username, code_hash)
  values (p_username, extensions.crypt(p_code, extensions.gen_salt('bf', 12)))
  returning * into v_customer;
  v_token := extensions.gen_random_bytes(32);
  insert into public.fizz_sessions(token_hash, customer_id, expires_at)
  values (extensions.digest(v_token, 'sha256'), v_customer.id, now() + interval '12 hours');
  return jsonb_build_object('username', v_customer.username, 'token', encode(v_token, 'hex'));
end;
$$;

create or replace function public.fizz_sign_in(p_username text, p_code text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_customer public.fizz_customers%rowtype;
  v_token bytea;
begin
  if p_username is null or p_code is null or p_code !~ '^[0-9]{6}$' then
    return jsonb_build_object('error', 'invalid_credentials');
  end if;
  select * into v_customer from public.fizz_customers
  where username = lower(trim(p_username)) for update;
  if not found then
    perform extensions.crypt(p_code, extensions.gen_salt('bf', 12));
    return jsonb_build_object('error', 'invalid_credentials');
  end if;
  if v_customer.locked_until > now() then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  if extensions.crypt(p_code, v_customer.code_hash) <> v_customer.code_hash then
    update public.fizz_customers
    set failed_attempts = case when v_customer.failed_attempts >= 4 then 0 else v_customer.failed_attempts + 1 end,
        locked_until = case when v_customer.failed_attempts >= 4 then now() + interval '15 minutes' else null end
    where id = v_customer.id;
    return jsonb_build_object('error', 'invalid_credentials');
  end if;
  update public.fizz_customers set failed_attempts = 0, locked_until = null where id = v_customer.id;
  v_token := extensions.gen_random_bytes(32);
  insert into public.fizz_sessions(token_hash, customer_id, expires_at)
  values (extensions.digest(v_token, 'sha256'), v_customer.id, now() + interval '12 hours');
  return jsonb_build_object('username', v_customer.username, 'token', encode(v_token, 'hex'));
end;
$$;

create or replace function public.fizz_current_customer(p_token text)
returns jsonb language sql stable security definer set search_path = '' as $$
  select jsonb_build_object('username', c.username)
  from public.fizz_sessions s
  join public.fizz_customers c on c.id = s.customer_id
  where p_token ~ '^[0-9a-f]{64}$'
    and s.token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256')
    and s.expires_at > now()
  limit 1;
$$;

create or replace function public.fizz_sign_out(p_token text)
returns void language plpgsql security definer set search_path = '' as $$
begin
  if p_token ~ '^[0-9a-f]{64}$' then
    delete from public.fizz_sessions
    where token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256');
  end if;
end;
$$;

revoke execute on function public.fizz_register(text, text), public.fizz_sign_in(text, text),
  public.fizz_current_customer(text), public.fizz_sign_out(text) from public, anon, authenticated;
grant execute on function public.fizz_register(text, text), public.fizz_sign_in(text, text),
  public.fizz_current_customer(text), public.fizz_sign_out(text) to service_role;
