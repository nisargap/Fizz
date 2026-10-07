-- Sign-in now goes only through Supabase Auth: Google, email and password, and passkeys.
-- Username and access-code accounts are removed together with everything they own (sensors,
-- readings, alerts, phones, agent tokens, and sessions all cascade).

delete from public.fizz_customers where auth_user_id is null;

drop function public.fizz_register(text, text, text, text);
drop function public.fizz_sign_in(text, text);

-- Every customer is a Supabase Auth user. Deleting that user (for example from the Supabase
-- dashboard when someone asks for account deletion) deletes the customer and its data.
alter table public.fizz_customers
  drop column code_hash,
  drop column failed_attempts,
  drop column locked_until,
  alter column auth_user_id set not null,
  drop constraint fizz_customers_auth_user_id_fkey,
  add constraint fizz_customers_auth_user_id_fkey
    foreign key (auth_user_id) references auth.users(id) on delete cascade;

-- Email sign-up checks the invite before Supabase creates the user. The invite is consumed later,
-- by fizz_oauth_sign_in, when the confirmed account is first used.
create function public.fizz_check_invite(p_invite text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_invite text := public.fizz_normalize_invite(p_invite);
begin
  if not public.fizz_rate_limit_hit('register_ip', p_ip, 10, interval '1 hour') then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  if v_invite is null or not exists (
    select 1 from public.fizz_invite_codes
    where code_hash = extensions.digest(v_invite, 'sha256')
      and used_at is null and (expires_at is null or expires_at > now())
  ) then
    return jsonb_build_object('error', 'invalid_invite');
  end if;
  return jsonb_build_object('ok', true);
end;
$$;

revoke execute on function public.fizz_check_invite(text, text) from public, anon, authenticated;
grant execute on function public.fizz_check_invite(text, text) to service_role;
