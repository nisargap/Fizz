create or replace function public.fizz_create_alert(p_token text, p_sensor_id uuid, p_metric text,
  p_comparator text, p_threshold double precision, p_unit text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_customer uuid; v_rule public.fizz_alert_rules%rowtype; v_unit text;
begin
  select customer_id into v_customer from public.fizz_sessions
  where p_token ~ '^[0-9a-f]{64}$'
    and token_hash = extensions.digest(decode(p_token, 'hex'), 'sha256') and expires_at > now();
  if v_customer is null then return jsonb_build_object('error','unauthorized'); end if;
  if not exists(select 1 from public.fizz_sensors where id = p_sensor_id
      and customer_id = v_customer and deleted_at is null) then
    return jsonb_build_object('error','sensor_not_found');
  end if;
  if p_metric is null or p_metric !~ '^[a-z][a-z0-9_]{0,63}$'
    or p_comparator not in ('gt','gte','lt','lte')
    or p_threshold is null or p_threshold = 'Infinity'::float8 or p_threshold = '-Infinity'::float8
    or p_unit is null or length(p_unit) > 24 then
    return jsonb_build_object('error','invalid_rule');
  end if;
  -- A rule can only target a metric/unit the sensor has actually reported.
  select coalesce(unit, '') into v_unit from public.fizz_sensor_readings
  where sensor_id = p_sensor_id and metric = p_metric and numeric_value is not null
  order by received_at desc limit 1;
  if not found or v_unit <> p_unit then
    return jsonb_build_object('error','unknown_metric');
  end if;
  insert into public.fizz_alert_rules(customer_id,sensor_id,metric,comparator,threshold,unit)
  values(v_customer,p_sensor_id,p_metric,p_comparator,p_threshold,p_unit) returning * into v_rule;
  return jsonb_build_object('rule',to_jsonb(v_rule)-'customer_id');
end;
$$;
