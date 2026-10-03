-- Keep all metrics in one event together. An invalid metric rolls back the
-- earlier inserts from the same event before returning a validation error.
create function public.fizz_ingest_event(p_sensor_id uuid, p_key text, p_event_id text,
  p_observed_at timestamptz, p_metrics jsonb)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_metric text; v_value jsonb; v_type text; v_result jsonb; v_results jsonb := '[]'::jsonb;
  v_count integer;
  v_numeric double precision; v_boolean boolean; v_text text; v_unit text;
begin
  if p_metrics is null or jsonb_typeof(p_metrics) <> 'object' then
    return jsonb_build_object('error','invalid_reading');
  end if;
  select count(*) into v_count from jsonb_object_keys(p_metrics);
  if v_count not between 1 and 16 then return jsonb_build_object('error','invalid_reading'); end if;
  begin
    for v_metric, v_value in select key,value from jsonb_each(p_metrics) loop
      v_type := jsonb_typeof(v_value);
      if v_metric !~ '^[a-z][a-z0-9_]{0,39}$'
        or v_type not in ('number','boolean','string') then
        raise exception 'invalid_reading' using errcode='22023';
      end if;
      v_numeric := null; v_boolean := null; v_text := null;
      if v_type = 'number' then v_numeric := (v_value #>> '{}')::double precision;
      elsif v_type = 'boolean' then v_boolean := (v_value #>> '{}')::boolean;
      else v_text := v_value #>> '{}'; end if;
      v_unit := case v_metric
        when 'flow_l_min' then 'L/min' when 'concentration_ppm' then 'ppm'
        when 'rssi_dbm' then 'dBm' when 'temperature_c' then '°C'
        when 'pressure_kpa' then 'kPa' when 'humidity_pct' then '%'
        when 'sound_db' then 'dB' when 'acceleration_ms2' then 'm/s²'
        when 'orientation_deg' then '°' else null end;
      v_result := public.fizz_ingest_reading(p_sensor_id,p_key,p_event_id,p_observed_at,
        v_metric,v_numeric,v_boolean,v_text,v_unit);
      if v_result ? 'error' then
        raise exception '%', v_result->>'error' using errcode='22023';
      end if;
      v_results := v_results || jsonb_build_array(v_result);
    end loop;
  exception when sqlstate '22023' then
    return jsonb_build_object('error',SQLERRM);
  end;
  return jsonb_build_object('accepted',v_results);
end;
$$;

revoke execute on function public.fizz_ingest_event(uuid,text,text,timestamptz,jsonb)
  from public, anon, authenticated;
grant execute on function public.fizz_ingest_event(uuid,text,text,timestamptz,jsonb)
  to service_role;

-- Multi-metric sensors should display their primary numeric signal on cards.
create or replace function public.fizz_sensor_json(p_sensor public.fizz_sensors)
returns jsonb language sql stable security definer set search_path = '' as $$
  select jsonb_build_object(
    'id', p_sensor.id, 'kind', p_sensor.kind, 'name', p_sensor.name,
    'mode', p_sensor.mode, 'status', p_sensor.status,
    'created_at', p_sensor.created_at,
    'latest_reading', (select to_jsonb(r) from public.fizz_sensor_readings r
      where r.sensor_id = p_sensor.id
      order by r.observed_at desc,
        case when r.metric = case p_sensor.kind
          when 'water' then 'flow_l_min' when 'gas' then 'concentration_ppm'
          when 'radio' then 'rssi_dbm' when 'temperature' then 'temperature_c'
          when 'pressure' then 'pressure_kpa' when 'humidity' then 'humidity_pct'
          when 'sound' then 'sound_db' when 'phone' then 'acceleration_ms2'
          else 'value' end then 0 else 1 end,
        r.received_at desc limit 1)
  );
$$;
