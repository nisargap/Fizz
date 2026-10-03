-- Only metadata is listed. Audio remains behind the owner-authenticated clip RPC.
create function public.fizz_phone_list_clips(p_owner_token text)
returns jsonb language plpgsql stable security definer set search_path = '' as $$
declare v_customer uuid;
begin
  v_customer := public.fizz_session_customer(p_owner_token);
  if v_customer is null then return jsonb_build_object('error', 'unauthorized'); end if;
  return jsonb_build_object('clips', coalesce((
    select jsonb_agg(to_jsonb(recent) order by recent.created_at desc, recent.clip_id desc)
    from (
      select c.id as clip_id, c.sensor_id, s.name as sensor_name,
             c.mime_type, c.created_at, octet_length(c.audio) as size_bytes
      from public.fizz_phone_clips c
      join public.fizz_sensors s on s.id = c.sensor_id
      where s.customer_id = v_customer and s.deleted_at is null
      order by c.created_at desc, c.id desc
      limit 20
    ) recent
  ), '[]'::jsonb));
end;
$$;

revoke all on function public.fizz_phone_list_clips(text) from public, anon, authenticated;
grant execute on function public.fizz_phone_list_clips(text) to service_role;
