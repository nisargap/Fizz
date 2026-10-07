-- Google sign-in through Supabase Auth. Supabase verifies the Google identity; Fizz links the
-- auth.users row to a customer and issues its usual hashed session token.

alter table public.fizz_customers
  add column auth_user_id uuid unique references auth.users(id) on delete set null,
  alter column code_hash drop not null;

-- Google-only customers have no access code, so a code must never sign them in. Without the
-- null check, crypt(p_code, null) <> null is null and the mismatch branch would be skipped.
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
  if not found or v_customer.code_hash is null then
    perform extensions.crypt(p_code, extensions.gen_salt('bf', 12));
    return jsonb_build_object('error', 'invalid_credentials');
  end if;
  if v_customer.locked_until > now() then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  if extensions.crypt(p_code, v_customer.code_hash) is distinct from v_customer.code_hash then
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

-- Called only after the Rust function exchanged a Supabase auth code, so p_user_id is a verified
-- auth.users id. Linked customers sign in; new ones need an unused invite and get a username
-- derived from their email address.
create function public.fizz_oauth_sign_in(p_user_id uuid, p_email text, p_invite text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_customer public.fizz_customers%rowtype;
  v_invite text := public.fizz_normalize_invite(p_invite);
  v_hash bytea;
  v_base text;
  v_username text;
  v_token bytea;
begin
  if p_user_id is null then
    return jsonb_build_object('error', 'unavailable');
  end if;
  select * into v_customer from public.fizz_customers where auth_user_id = p_user_id;
  if not found then
    if v_invite is null then
      return jsonb_build_object('error', 'no_account');
    end if;
    if not public.fizz_rate_limit_hit('register_ip', p_ip, 10, interval '1 hour') then
      return jsonb_build_object('error', 'rate_limited');
    end if;
    v_hash := extensions.digest(v_invite, 'sha256');
    perform 1 from public.fizz_invite_codes
    where code_hash = v_hash and used_at is null and (expires_at is null or expires_at > now())
    for update;
    if not found then
      return jsonb_build_object('error', 'invalid_invite');
    end if;
    v_base := regexp_replace(lower(split_part(coalesce(p_email, ''), '@', 1)), '[^a-z0-9_]', '', 'g');
    if v_base !~ '^[a-z]' then v_base := 'fizz_' || v_base; end if;
    v_base := left(v_base, 18);
    if length(v_base) < 3 then v_base := 'fizz_user'; end if;
    for i in 0..9 loop
      v_username := case when i = 0 then v_base
        else v_base || '_' || lpad(floor(random() * 10000)::int::text, 4, '0') end;
      begin
        insert into public.fizz_customers(username, auth_user_id)
        values (v_username, p_user_id)
        returning * into v_customer;
        exit;
      exception when unique_violation then
        null;
      end;
    end loop;
    if v_customer.id is null then
      return jsonb_build_object('error', 'unavailable');
    end if;
    update public.fizz_invite_codes set used_at = now(), used_by = v_customer.id where code_hash = v_hash;
  end if;
  v_token := extensions.gen_random_bytes(32);
  insert into public.fizz_sessions(token_hash, customer_id, expires_at)
  values (extensions.digest(v_token, 'sha256'), v_customer.id, now() + interval '12 hours');
  return jsonb_build_object('username', v_customer.username, 'token', encode(v_token, 'hex'));
end;
$$;

revoke execute on function public.fizz_oauth_sign_in(uuid, text, text, text) from public, anon, authenticated;
grant execute on function public.fizz_oauth_sign_in(uuid, text, text, text) to service_role;
