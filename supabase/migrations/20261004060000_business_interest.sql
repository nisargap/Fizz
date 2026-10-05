-- Businesses register interest separately from the consumer waitlist, with a company name and
-- an optional note about what they want to monitor. Submitting again updates the entry.
create table public.fizz_business_interest (
  id uuid primary key default gen_random_uuid(),
  email text not null unique check (
    length(email) between 3 and 254 and email = lower(email) and email ~ '^[^@\s]+@[^@\s]+\.[^@\s]+$'
  ),
  company text not null check (length(company) between 1 and 120),
  interest text check (length(interest) <= 500),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

alter table public.fizz_business_interest enable row level security;
revoke all on public.fizz_business_interest from anon, authenticated;
grant all on public.fizz_business_interest to service_role;

-- Same answer for new and repeat emails, so the list cannot be probed.
create function public.fizz_join_business_interest(p_email text, p_company text, p_interest text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_email text := lower(trim(coalesce(p_email, '')));
  v_company text := trim(coalesce(p_company, ''));
  v_interest text := nullif(trim(coalesce(p_interest, '')), '');
begin
  if not public.fizz_rate_limit_hit('business_ip', p_ip, 10, interval '1 hour')
    or not public.fizz_rate_limit_hit('business_all', 'all', 1000, interval '1 hour') then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  if length(v_email) not between 3 and 254 or v_email !~ '^[^@\s]+@[^@\s]+\.[^@\s]+$' then
    return jsonb_build_object('error', 'invalid_email');
  end if;
  if length(v_company) not between 1 and 120 or length(coalesce(v_interest, '')) > 500 then
    return jsonb_build_object('error', 'invalid_details');
  end if;
  insert into public.fizz_business_interest as b (email, company, interest) values (v_email, v_company, v_interest)
  on conflict (email) do update
    set company = excluded.company, interest = coalesce(excluded.interest, b.interest),
        updated_at = now();
  return jsonb_build_object('ok', true);
end;
$$;

revoke execute on function public.fizz_join_business_interest(text, text, text, text) from public, anon, authenticated;
grant execute on function public.fizz_join_business_interest(text, text, text, text) to service_role;
