create function public.fizz_remove_sensor_choice(p_token text, p_kind text)
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
  delete from public.fizz_sensor_choices
  where customer_id = v_customer_id and kind = p_kind;
  return public.fizz_get_sensor_choices(p_token);
end;
$$;

revoke execute on function public.fizz_remove_sensor_choice(text, text) from public, anon, authenticated;
grant execute on function public.fizz_remove_sensor_choice(text, text) to service_role;
