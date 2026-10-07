-- The Settings page reads account details and passkeys from Supabase Auth's admin API, which is
-- keyed by the Supabase user. This maps a session token to that user. It is for the server only.
create function public.fizz_session_account(p_token text)
returns jsonb language sql stable security definer set search_path = '' as $$
  select jsonb_build_object('username', c.username, 'auth_user_id', c.auth_user_id)
  from public.fizz_sessions s
  join public.fizz_customers c on c.id = s.customer_id
  where p_token ~ '^[0-9a-f]{64}$'
    and s.token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256')
    and s.expires_at > now()
  limit 1;
$$;

revoke execute on function public.fizz_session_account(text) from public, anon, authenticated;
grant execute on function public.fizz_session_account(text) to service_role;
