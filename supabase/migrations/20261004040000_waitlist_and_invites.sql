-- Lock down sign-up: the public joins an email waitlist, and new accounts need a single-use
-- invite code. Existing customers keep signing in as before.

-- Fixed-window counters keyed by a hashed subject (such as a client IP). Rows are tiny and
-- pruned opportunistically, so no scheduled job is needed.
create table public.fizz_rate_limits (
  bucket text not null,
  subject_hash bytea not null,
  window_start timestamptz not null,
  hits integer not null default 0,
  primary key (bucket, subject_hash, window_start)
);
create index fizz_rate_limits_window_start_idx on public.fizz_rate_limits(window_start);

create table public.fizz_waitlist (
  id uuid primary key default gen_random_uuid(),
  email text not null unique check (
    length(email) between 3 and 254 and email = lower(email) and email ~ '^[^@\s]+@[^@\s]+\.[^@\s]+$'
  ),
  created_at timestamptz not null default now()
);

-- Only a SHA-256 hash of each normalized code is stored; the plaintext is shown once when minted.
create table public.fizz_invite_codes (
  code_hash bytea primary key,
  note text check (length(note) <= 200),
  created_at timestamptz not null default now(),
  expires_at timestamptz,
  used_at timestamptz,
  used_by uuid references public.fizz_customers(id) on delete set null
);

alter table public.fizz_rate_limits enable row level security;
alter table public.fizz_waitlist enable row level security;
alter table public.fizz_invite_codes enable row level security;
revoke all on public.fizz_rate_limits, public.fizz_waitlist, public.fizz_invite_codes from anon, authenticated;
grant all on public.fizz_rate_limits, public.fizz_waitlist, public.fizz_invite_codes to service_role;

-- Count one hit for the subject and report whether it is still within the limit. Rejected
-- hits are counted too, so a client that keeps retrying stays blocked for the window.
create function public.fizz_rate_limit_hit(p_bucket text, p_subject text, p_limit integer, p_window interval)
returns boolean language plpgsql security definer set search_path = '' as $$
declare
  v_seconds double precision := extract(epoch from p_window);
  v_start timestamptz := to_timestamp(floor(extract(epoch from now()) / v_seconds) * v_seconds);
  v_hits integer;
begin
  insert into public.fizz_rate_limits as r (bucket, subject_hash, window_start, hits)
  values (p_bucket, extensions.digest(coalesce(p_subject, ''), 'sha256'), v_start, 1)
  on conflict (bucket, subject_hash, window_start) do update set hits = r.hits + 1
  returning hits into v_hits;
  if random() < 0.02 then
    delete from public.fizz_rate_limits where window_start < now() - interval '2 days';
  end if;
  return v_hits <= p_limit;
end;
$$;

-- Accept FIZZ-ABCD-EFGH-JKMN in any case, with or without the prefix, dashes, or spaces.
-- Crockford base32 reads O as 0 and I or L as 1, so hand-typed codes still match.
create function public.fizz_normalize_invite(p_code text)
returns text language plpgsql immutable set search_path = '' as $$
declare v_code text := regexp_replace(upper(coalesce(p_code, '')), '[^A-Z0-9]', '', 'g');
begin
  if length(v_code) = 16 and left(v_code, 4) = 'FIZZ' then v_code := substr(v_code, 5); end if;
  v_code := translate(v_code, 'OIL', '011');
  if v_code !~ '^[0-9A-HJKMNP-TV-Z]{12}$' then return null; end if;
  return v_code;
end;
$$;

-- Mint invite codes (service role only). Run from the Supabase SQL editor, for example:
--   select * from public.fizz_create_invites(5, 'Hackathon judges', interval '14 days');
create function public.fizz_create_invites(p_count integer default 1, p_note text default null,
  p_expires_in interval default null)
returns table(code text) language plpgsql security definer set search_path = '' as $$
declare
  v_alphabet constant text := '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
  v_bytes bytea;
  v_raw text;
begin
  if p_count is null or p_count not between 1 and 100 then
    raise exception 'Create between 1 and 100 invites at a time' using errcode = '22023';
  end if;
  for i in 1..p_count loop
    -- 12 symbols from 32 = 60 random bits; 256 is a multiple of 32, so byte % 32 is unbiased.
    v_bytes := extensions.gen_random_bytes(12);
    v_raw := '';
    for j in 0..11 loop
      v_raw := v_raw || substr(v_alphabet, get_byte(v_bytes, j) % 32 + 1, 1);
    end loop;
    insert into public.fizz_invite_codes(code_hash, note, expires_at)
    values (extensions.digest(v_raw, 'sha256'), p_note, now() + p_expires_in);
    code := 'FIZZ-' || substr(v_raw, 1, 4) || '-' || substr(v_raw, 5, 4) || '-' || substr(v_raw, 9, 4);
    return next;
  end loop;
end;
$$;

-- Duplicate emails get the same answer as new ones so the list cannot be probed.
create function public.fizz_join_waitlist(p_email text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_email text := lower(trim(coalesce(p_email, '')));
begin
  if not public.fizz_rate_limit_hit('waitlist_ip', p_ip, 5, interval '1 hour')
    or not public.fizz_rate_limit_hit('waitlist_all', 'all', 500, interval '1 hour') then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  if length(v_email) not between 3 and 254 or v_email !~ '^[^@\s]+@[^@\s]+\.[^@\s]+$' then
    return jsonb_build_object('error', 'invalid_email');
  end if;
  insert into public.fizz_waitlist(email) values (v_email) on conflict (email) do nothing;
  return jsonb_build_object('ok', true);
end;
$$;

-- Registration now consumes an invite in the same transaction that creates the account.
-- Errors are returned rather than raised so rate-limit hits are kept for failed attempts.
drop function public.fizz_register(text, text);
create function public.fizz_register(p_username text, p_code text, p_invite text, p_ip text)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  v_invite text := public.fizz_normalize_invite(p_invite);
  v_hash bytea;
  v_customer public.fizz_customers%rowtype;
  v_token bytea;
begin
  if not public.fizz_rate_limit_hit('register_ip', p_ip, 10, interval '1 hour') then
    return jsonb_build_object('error', 'rate_limited');
  end if;
  p_username := lower(trim(p_username));
  if p_username !~ '^[a-z][a-z0-9_]{2,23}$' or p_code !~ '^[0-9]{6}$' then
    return jsonb_build_object('error', 'invalid_credentials');
  end if;
  if v_invite is null then
    return jsonb_build_object('error', 'invalid_invite');
  end if;
  v_hash := extensions.digest(v_invite, 'sha256');
  perform 1 from public.fizz_invite_codes
  where code_hash = v_hash and used_at is null and (expires_at is null or expires_at > now())
  for update;
  if not found then
    return jsonb_build_object('error', 'invalid_invite');
  end if;
  begin
    insert into public.fizz_customers(username, code_hash)
    values (p_username, extensions.crypt(p_code, extensions.gen_salt('bf', 12)))
    returning * into v_customer;
  exception when unique_violation then
    return jsonb_build_object('error', 'username_taken');
  end;
  update public.fizz_invite_codes set used_at = now(), used_by = v_customer.id where code_hash = v_hash;
  v_token := extensions.gen_random_bytes(32);
  insert into public.fizz_sessions(token_hash, customer_id, expires_at)
  values (extensions.digest(v_token, 'sha256'), v_customer.id, now() + interval '12 hours');
  return jsonb_build_object('username', v_customer.username, 'token', encode(v_token, 'hex'));
end;
$$;

revoke execute on function public.fizz_rate_limit_hit(text, text, integer, interval),
  public.fizz_normalize_invite(text), public.fizz_create_invites(integer, text, interval),
  public.fizz_join_waitlist(text, text), public.fizz_register(text, text, text, text)
  from public, anon, authenticated;
grant execute on function public.fizz_create_invites(integer, text, interval),
  public.fizz_join_waitlist(text, text), public.fizz_register(text, text, text, text)
  to service_role;
