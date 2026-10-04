-- Raise waitlist limits for launch traffic: shared networks (offices, conferences, carrier NAT)
-- get 20 sign-ups per hour, and the global cap of 5,000 per hour still stops bulk abuse.
create or replace function public.fizz_join_waitlist(p_email text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_email text := lower(trim(coalesce(p_email, '')));
begin
  if not public.fizz_rate_limit_hit('waitlist_ip', p_ip, 20, interval '1 hour')
    or not public.fizz_rate_limit_hit('waitlist_all', 'all', 5000, interval '1 hour') then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  if length(v_email) not between 3 and 254 or v_email !~ '^[^@\s]+@[^@\s]+\.[^@\s]+$' then
    return jsonb_build_object('error', 'invalid_email');
  end if;
  insert into public.fizz_waitlist(email) values (v_email) on conflict (email) do nothing;
  return jsonb_build_object('ok', true);
end;
$$;

revoke execute on function public.fizz_join_waitlist(text, text) from public, anon, authenticated;
grant execute on function public.fizz_join_waitlist(text, text) to service_role;
