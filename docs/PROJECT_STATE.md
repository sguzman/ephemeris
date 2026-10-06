# Project State

Last verified implementation milestone: 2026-10-06.

## Current status

Ephemeris is now a working native Rust + egui calendar foundation backed by an event-native SQLite temporal store.

The original documentation/inheritance phase is complete. The project has crossed the important architectural boundary that motivated the spinoff from Rivetr: calendar data is no longer forced through `TaskDto` or task-file persistence.

The current application can ingest real Taria Resourcearium reconciled temporal event sets and compact recovered reconciled indexes, preserve rich upstream temporal semantics and provenance references, query the resulting corpus, render standard calendar ranges, inspect records, and persist named programmable calendar views. Taria has established immutable TemporalBundleRelease packaging, and Ephemeris now has the first filesystem-first release consumer slice: local Resourcearium discovery, local channel/manifest resolution, hash validation, and one-click supported-shard adoption.

## Verified quality gate

The 2026-10-04 milestone passes:

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```

At the hardened Phase 8 recurrence + RFC 5545 adapter checkpoint, all **447** library tests pass. Verified implementation code checkpoint: `d5e5bfd50e52131ab42a4b7219b0164a1e5b2381`, with format, check, strict Clippy, and tests all green.

`main` is verified through BYSECOND at `c5ce73bb86cf3c2281be69adcf15b93364f3f65a`: format, check, strict Clippy, and all **265** library tests pass in GitHub Actions. The preceding BYMINUTE checkpoint is independently verified at `1d2ce529facad7636be6f136740976351910e253` with **253** tests.

BYHOUR, BYMINUTE, and BYSECOND expansion are verified across DAILY/WEEKLY/MONTHLY/YEARLY floating/exact date-time recurrence. They compose in hour→minute→second order before BYSETPOS and COUNT, preserve fractional-second/duration and source-local DST semantics, give reachability the full hour×minute×second candidate cardinality, and preserve time-specific occurrence identity/override targeting. The fields remain schema-v14 compatible through serde defaults. RFC 5545 leap-second selector `BYSECOND=60` is explicitly rejected until the canonical time model can represent leap seconds faithfully rather than normalizing them.

HOURLY frequency is verified on `main` at `fffc94ae4d65ddef52581294d0a8c0a935ffd23c`: format, check, strict Clippy, and all **278** library tests pass in GitHub Actions. HOURLY requires floating/exact date-time DTSTART, advances by source-local wall-clock hours, treats BYMONTH/BYYEARDAY/BYMONTHDAY/plain-BYDAY/BYHOUR as limiters, expands BYMINUTE/BYSECOND, then applies BYSETPOS/COUNT. BYHOUR-only reachability uses a 24-hour fast path; rules with calendar limiters scan the finite 400-year / 3,506,328-hour Gregorian cycle, so permanently empty hourly selector/cadence combinations terminate. The verified slice also covers DST-gap skipping, same-day timestamp-aware override validation, persistence round-trip coverage, and remains schema-v14 compatible because the new frequency value is stored inside the existing recurrence JSON.

MINUTELY frequency is verified on `main` at `abd6438a1c14e8fbbf18ded110caf373cbda99de`: format, check, strict Clippy, and all **293** library tests pass in GitHub Actions. MINUTELY requires floating/exact date-time DTSTART, advances by source-local wall-clock minutes, treats BYMONTH/BYYEARDAY/BYMONTHDAY/plain-BYDAY/BYHOUR/BYMINUTE as limiters, expands BYSECOND, then applies BYSETPOS/COUNT. Time-of-day-only reachability uses the finite 1,440-minute daily cycle; when calendar limiters participate, allowed hour/minute combinations are reduced once to cadence residues and the finite 400-year Gregorian pass scans only 146,097 civil dates rather than every minute. The verified slice includes DST-gap skipping, same-hour timestamp-aware override validation, explicit EXDATE+moved-override identity coverage, persistence/date-only rejection coverage, and remains schema-v14 compatible.

SECONDLY frequency is verified on `main` at `1d109a5fc2cab8d758d07e9b25fb7574f9de3614`: format, check, strict Clippy, and all **307** library tests pass in GitHub Actions. SECONDLY requires floating/exact date-time DTSTART, advances by source-local wall-clock seconds, treats BYMONTH/BYYEARDAY/BYMONTHDAY/plain-BYDAY/BYHOUR/BYMINUTE/BYSECOND as limiters, then applies BYSETPOS/COUNT. Time-only reachability uses the finite 86,400-second daily cycle; calendar-limited reachability reduces allowed wall-clock times to modular residues and scans the 146,097-day Gregorian cycle instead of the 12.6-billion-second full cycle. The verified slice covers DST-gap skipping, timestamp-aware same-minute override validation, EXDATE+moved-override identity, persistence/date-only rejection, and remains schema-v14 compatible.

The first recurrence-authoring slice is verified on `main` at `5ad550821fba6b7c6b5bdee7acb3afaab11b8ea6`: format, check, strict Clippy, and all **311** library tests pass in GitHub Actions. The inspector-launched canonical-series editor edits frequency, interval, COUNT, UNTIL, WKST, plain BYDAY, and BYMONTH; validates through the existing recurrence domain model before persistence; writes the canonical event rather than a materialized occurrence; and refuses edits for read-only sources.

The advanced selector-authoring slice is verified on `main` at `680b7b55f22dab7d4125f2db7f49d571ba250ea7`: format, check, strict Clippy, and all **315** library tests pass in GitHub Actions. It adds direct editing for BYWEEKNO, BYYEARDAY, BYMONTHDAY, ordinal BYDAY, BYHOUR, BYMINUTE, BYSECOND, and BYSETPOS using compact comma/space-separated typed fields with live domain validation.

The exception-date authoring slice is verified on `main` at `174ffa0a6485a21211adc20d83b81a42b8dba075`: format, check, strict Clippy, and all **320** library tests pass in GitHub Actions. It adds direct RDATE/EXDATE editing. Exception inputs are authored as occurrence starts in the canonical series' temporal kind, reconstruct canonical duration/source-timezone metadata on edited entries, preserve untouched exception payloads at the domain-object level, and validate EXDATE/override conflicts live through canonical recurrence validation.

The occurrence-override authoring slice is verified on `main` at `1db27c65042e579730071d37e9e1b883ef4c72b3`: format, check, strict Clippy, and all **326** library tests pass in GitHub Actions. It adds multiline editing for moved, cancelled, cancelled+moved, and explicit no-op overrides using `original=>replacement`, `original=>CANCEL`, `original=>CANCEL@replacement`, and `original=>KEEP`; parsed endpoints inherit canonical temporal kind/duration/source-timezone metadata and the existing domain validator rejects malformed, duplicate, conflicting, or phantom targets before save. Together with the verified RDATE/EXDATE editor, the compact canonical recurrence-exception authoring boundary is now complete.

The structured occurrence-override editor is verified on `main` at `6d333cbbe85b5aaa9b62804e5951394444625d84`: format, check, strict Clippy, and all **329** library tests pass in GitHub Actions. Move, Cancel, Cancel + move, and Keep are explicit per-row actions with original/replacement fields and add/remove controls. The compact syntax remains available in a collapsed raw editor and still flows through the same parser/domain validator, so structured editing does not create a second recurrence model. Untouched persisted override payloads continue to be preserved at the domain-object level.

The structured RDATE/EXDATE editor is verified on `main` at `3aaaa875a4cd66fb4e3727182469b7d1ce78547b`: format, check, strict Clippy, and all **333** library tests pass in GitHub Actions. Each addition/exclusion is edited as an explicit occurrence-start row with add/remove controls and a collapsed raw compact-syntax fallback. Blank unfinished rows fail live validation instead of disappearing silently, and untouched persisted exception payloads remain unchanged at the domain-object level. Together with the 329-test structured override slice, recurrence exceptions are now fully structured while retaining raw power-user syntax.

The canonical recurrence preset slice is verified on `main` at `b97225fcc464041f0015ba4b21dc236eeb105b86`: format, check, strict Clippy, and all **337** library tests pass in GitHub Actions. Daily, Weekdays, Weekly, Monthly, Last weekday/month, and Yearly presets reset only cadence selectors plus interval to known canonical rules while preserving COUNT, UNTIL, RDATE, EXDATE, and occurrence overrides.

The contextual recurrence-editor selector slice is verified on `main` at `56210f55371eb7ec077207ea81e3cee749e961e5`: format, check, strict Clippy, and all **342** library tests pass in GitHub Actions. Selector rows follow the domain frequency matrix and canonical event time kind; inapplicable empty fields are hidden, populated incompatible fields remain explicitly preserved with a Clear action, custom WKST appears only in real weekly/yearly week contexts, and sub-daily frequencies are omitted for date/all-day series.

The structured ordinal-BYDAY editor is verified on `main` at `6c7f51b15410fd38606734e4ebfe9b6dd2b81a53`: format, check, strict Clippy, and all **347** library tests pass in GitHub Actions. Month/year ordinal weekday selectors are edited as explicit ordinal + weekday rows with add/remove controls; compact tokens such as `1MO,-1FR` remain available in a collapsed raw editor, raw edits stay authoritative until loaded into rows, and presets/incompatible-context clearing keep both representations synchronized.

The structured BYMONTHDAY editor is verified on `main` at `1ca746f9e486a68ffdb64d91320f08faa06ed4f0`: format, check, strict Clippy, and all **352** library tests pass in GitHub Actions. Signed month-day selectors are edited as one civil-day row per value with add/remove controls; positive values count from month start, negative values from month end, and compact signed syntax such as `1,15,-1` remains available in a collapsed raw editor with the same raw/structured authority and synchronization rules.

The structured BYWEEKNO editor is verified on `main` at `0dea3c2f151f2cfbdcb78646a478c7eba89c23d3`: format, check, strict Clippy, and all **357** library tests pass in GitHub Actions. Each signed week selector is an explicit row with add/remove controls; raw forms such as `20,-1` remain available in a collapsed fallback, and raw/structured authority, presets, contextual preservation, and clearing follow the same non-parallel editing contract.

The structured BYYEARDAY editor is verified on `main` at `1c9f319491d879696f04239105b867b5d0597b7f`: format, check, strict Clippy, and all **362** library tests pass in GitHub Actions. Signed year-day selectors are edited as one row per positive/negative civil-year position with add/remove controls; compact syntax such as `1,100,-1` remains available in a collapsed raw fallback with the same authority, preset, contextual-preservation, and clearing rules.

The structured BYHOUR editor is verified on `main` at `af24ca5196712cef0a9290e033bc41b0d9cdbfdf`: format, check, strict Clippy, and all **367** library tests pass in GitHub Actions. Each civil hour is an explicit row with add/remove controls; compact forms such as `9,17` remain available in a collapsed raw fallback, and contextual preservation/clearing plus raw-vs-structured authority follow the same non-parallel editing contract.

The structured BYMINUTE editor is verified on `main` at `ca59649d3fa60d72c047b5fdfaf308e1c007ac1e`: format, check, strict Clippy, and all **372** library tests pass in GitHub Actions. Each civil minute is an explicit row with add/remove controls; compact forms such as `0,30` remain available in a collapsed raw fallback, and contextual preservation/clearing plus raw-vs-structured authority follow the same non-parallel editing contract.

The structured BYSECOND editor is verified on `main` at `406ff4efa656c689f8c4b307fad4bdae99fdc57a`: format, check, strict Clippy, and all **377** library tests pass in GitHub Actions. Each ordinary civil second is an explicit row with add/remove controls; compact forms such as `0,15,30,45` remain available in a collapsed raw fallback, contextual preservation/clearing plus raw-vs-structured authority follow the same non-parallel editing contract, and selector value `60` remains rejected by the existing leap-second-safe domain validation.

The structured BYSETPOS editor is verified on `main` at `6484377a1cbd60a954c3d97851603f9ff9213f60`: format, check, strict Clippy, and all **382** library tests pass in GitHub Actions. Signed set positions are edited one per row; new rows default to `1` because zero is invalid, compact forms such as `1,-1` remain available in a collapsed raw fallback, and the Last weekday/month preset synchronizes its canonical `-1` selector into both raw and structured state. The obsolete generic raw-selector row helpers are removed because every advanced numeric BY selector now has a dedicated structured editor.

The strict RFC 5545 RRULE adapter is verified on `main` at `a9eacd509b3c7c2f4e58e4ed2842154b88093edc`: format, check, strict Clippy, and all **395** library tests pass in GitHub Actions. The adapter parses bare RECUR values or `RRULE:` properties, emits stable FREQ-first RRULE text, maps every currently supported canonical selector, rejects duplicate/unknown parts and COUNT+UNTIL combinations, safely rejects malformed/non-ASCII BYDAY tokens, delegates selector legality to domain validation, and rejects DATE-TIME UNTIL rather than truncating it into Ephemeris' civil-date bound.

The RFC 5545 RDATE/EXDATE adapter is verified on `main` at `41e8aa62a090df82751506cf2ab30436563d936a`: format, check, strict Clippy, and all **407** library tests pass in GitHub Actions. DATE and all-day series preserve their distinct canonical kind and duration; floating exceptions preserve local wall time and source metadata; exact exceptions accept UTC `Z` values or a TZID matching the master source timezone, with RFC-compatible first-occurrence handling for ambiguous local times and pre-gap-offset handling for nonexistent local times. Duplicate recurrence-date values collapse deterministically. RDATE `VALUE=PERIOD`, unknown value types/parameters, incompatible kinds/timezones, mismatched durations/source metadata, and fractional-second exports that RFC basic DATE-TIME cannot preserve are rejected instead of normalized.

DAILY reachability switches to the finite 400-year Gregorian date cycle whenever BYMONTH or BYMONTHDAY is present, while BYDAY-only rules retain the smaller weekday-cycle fast path. YEARLY reachability likewise scans every distinct state in the 400-year Gregorian cycle using the existing yearly candidate generator, including BYSETPOS, and deliberately evaluates future-cycle periods so first-year DTSTART filtering cannot create false negatives.

## Implemented architecture

### Native application

- Rust 2024
- `eframe` / `egui`
- no Tauri
- no React/WebView shell
- native local UI-state persistence

### Canonical local store

SQLite via bundled `rusqlite`.

Current schema version: 14.

The database owns:

- temporal sources
- temporal events
- durable saved views
- Taria source/import-record mappings
- Taria upstream event/reconciled identity aliases
- immutable Taria release metadata
- release-to-source projection associations
- immutable per-release canonical event snapshots
- immutable CalendarSets
- release-to-CalendarSet associations
- projected calendars
- CalendarSet event memberships
- persisted event recurrence definitions

Transient UI state remains separate.

### Canonical temporal model

Implemented time forms:

- date-only civil date/range
- explicit all-day date/range
- exact UTC instant with retained source-timezone context
- floating/local date-time
- month precision
- year precision
- unresolved/blocked temporal value

Month/year precision is not coerced onto an invented day. Date-only data is not silently promoted to an instant or conflated with explicit all-day semantics.

### Recurrence foundation

Schema v14 adds an optional recurrence definition to the canonical event record.

Implemented:

- secondly, minutely, hourly, daily, weekly, monthly, and yearly frequency;
- positive interval;
- optional occurrence count;
- optional inclusive civil-date `until` bound;
- secondly/minutely/hourly plain-BYDAY limiting over active cadence timestamps and daily BYDAY filtering over active interval dates, with skipped candidates not consuming COUNT;
- secondly selector reachability using an 86,400-second time-of-day fast path and modular residue matching over the 400-year Gregorian date cycle when calendar limiters participate;
- minutely selector reachability using a 1,440-minute time-of-day fast path and modular congruence over the 400-year Gregorian minute cycle when calendar limiters participate;
- hourly selector reachability using a 24-hour fast path for BYHOUR-only rules and the finite 400-year / 3,506,328-hour Gregorian cycle when calendar limiters are present;
- daily BYMONTH limiting over active interval dates, intersecting with daily BYDAY when both are present and using a 400-year Gregorian-cycle reachability guard for permanently empty selector/interval combinations;
- signed daily BYMONTHDAY limiting for civil days `-31..=-1` and `1..=31`, intersecting with DAILY BYMONTH/BYDAY and sharing the same finite Gregorian-cycle reachability guard;
- monthly BYMONTH limiting over active recurrence months, with a 4,800-month Gregorian-cycle reachability guard that also detects permanently empty monthly BYDAY/BYMONTHDAY/ordinal-BYDAY/BYSETPOS combinations;
- weekly BYMONTH limiting over generated weekly dates, applied after BYDAY expansion and before BYSETPOS, with a 20,871-week Gregorian-cycle reachability guard;
- yearly selector reachability over the finite 400-year Gregorian cycle, covering permanently empty month/day/year-day/week-number/BYDAY/BYSETPOS combinations while preserving future-cycle candidates hidden by first-year DTSTART filtering;
- weekly multi-day BYDAY selection with duplicate weekday validation;
- explicit WKST recurrence-week anchoring for weekly BYDAY, independent from the user's display-week preference;
- chronological weekday generation within each WKST-anchored active recurrence week, with first-week candidates before DTSTART omitted;
- plain monthly BYDAY selection, expanding every matching weekday inside each active month while leaving WKST semantically inactive for monthly rules;
- signed BYMONTHDAY selection for civil days `-31..=-1` and `1..=31`: SECONDLY, MINUTELY, HOURLY, and DAILY limit active cadence dates, MONTHLY resolves dates inside the active month, YEARLY expands/filters inside recurrence years, and WEEKLY remains invalid; zero/duplicate/range validation applies consistently;
- positive BYMONTHDAY values count from month start, negative values count backward from month end (`-1` = last day), and selector aliases resolving to the same civil date are deduplicated;
- chronological resolved-date generation within each active month, with first-month candidates before DTSTART omitted and impossible civil dates skipped;
- monthly ordinal BYDAY selection for first-through-fifth or last-through-fifth-from-last weekdays (`±1..±5`), with zero/out-of-range/duplicate validation;
- yearly ordinal BYDAY is context-aware: with BYMONTH it remains month-scoped at `±1..±5`; without BYMONTH it resolves the nth weekday of the whole recurrence year at `±1..±53`; numeric BYDAY remains invalid with BYWEEKNO;
- ordinal weekday candidates resolved chronologically inside each active month, with missing fifth weekdays skipped and first-month candidates before DTSTART omitted;
- plain and ordinal monthly BYDAY selectors form one resolved BYDAY civil-date union; when BYMONTHDAY is present it filters that union before BYSETPOS/COUNT/EXDATE/override processing;
- the canonical last-weekday rule (`MO,TU,WE,TH,FR` + `BYSETPOS=-1`) is therefore representable directly;
- positive BYMONTH selection for months 1-12 in all supported frequencies: SECONDLY/MINUTELY/HOURLY/DAILY/WEEKLY/MONTHLY limit generated cadence candidates, while YEARLY expands the active month set; duplicate and range validation remains explicit;
- yearly plain BYDAY expansion for every selected weekday across the active recurrence year, or only inside selected BYMONTH months when BYMONTH is present;
- signed yearly BYMONTHDAY expansion across every month when BYMONTH is absent, or across selected months when BYMONTH is present, with positive or month-end-relative day selectors;
- yearly BYMONTH + ordinal BYDAY composition, where ordinal weekdays are resolved inside each selected month; without BYMONTH, the same persisted ordinal selector resolves against the whole recurrence year;
- when yearly BYMONTHDAY and ordinal BYDAY are both present, Ephemeris intersects their resolved civil-date sets independently inside each selected month before COUNT/EXDATE/override processing;
- yearly BYMONTHDAY is valid with or without BYMONTH; ordinal BYDAY is likewise valid in yearly rules but is context-aware, resolving within selected months when BYMONTH is present and against the whole recurrence year when BYMONTH is absent;
- signed yearly BYWEEKNO selection for `-53..=-1` and `1..=53`, with zero/out-of-range/duplicate/non-yearly validation;
- BYWEEKNO uses WKST-aware seven-day weeks; week 1 is the WKST-anchored week containing January 4, and negative week numbers count backward from the final numbered week;
- yearly BYWEEKNO accepts plain BYDAY weekdays inside selected week-number sets; without BYDAY, DTSTART's weekday is preserved inside each selected week;
- custom WKST is valid for weekly BYDAY and yearly BYWEEKNO contexts, while ordinal BYDAY remains invalid with BYWEEKNO;
- week 53 is skipped in week-number years that contain only 52 weeks, and BYMONTH/BYYEARDAY plus valid BYMONTHDAY context filter week-number candidates before BYSETPOS/COUNT/exceptions;
- signed BYYEARDAY selection for `-366..=-1` and `1..=366`: SECONDLY/MINUTELY/HOURLY limit active cadence dates while YEARLY resolves/expands selected year-days; zero/out-of-range/duplicate/unsupported-frequency validation applies;
- positive BYYEARDAY values count from January 1 and negative values count backward from year-end (`-1` = December 31); day 366 is skipped in non-leap years rather than coerced;
- when BYYEARDAY is combined with BYMONTH, plain/ordinal BYDAY, and existing month-scoped selectors, those selectors filter the resolved year-day set before COUNT/EXDATE/override processing; plain and ordinal BYDAY forms remain one unioned BYDAY family;
- SECONDLY, MINUTELY, and HOURLY frequencies over floating/exact date-time bases, with source-local wall-clock cadence and frequency-aware limiting/expanding BY-part semantics;
- BYHOUR expansion for `0..=23` on DAILY/WEEKLY/MONTHLY/YEARLY floating/exact date-time bases; SECONDLY/MINUTELY/HOURLY use BYHOUR as a limiter, preserving DTSTART minute/second/duration and source-local DST semantics; DATE/all-day bases are rejected;
- BYMINUTE expansion for `0..=59` on HOURLY/DAILY/WEEKLY/MONTHLY/YEARLY floating/exact date-time bases, while SECONDLY/MINUTELY use BYMINUTE as a limiter; DTSTART second/duration and DATE/all-day rejection semantics are preserved;
- BYSECOND handling for ordinary civil seconds `0..=59` on floating/exact date-time bases: SECONDLY limits the active cadence second, while MINUTELY/HOURLY/DAILY/WEEKLY/MONTHLY/YEARLY expand selected seconds at their frequency-appropriate stage; fractional-second/duration and DATE/all-day rejection semantics are preserved, and RFC 5545 leap-second selector `60` remains explicitly unsupported rather than coerced;
- generic signed BYSETPOS selection for `-366..=-1` and `1..=366`, applied to each recurrence interval's fully resolved BY-selector candidate set after frequency-aware BYHOUR/BYMINUTE/BYSECOND handling and before COUNT is consumed; SECONDLY/MINUTELY/HOURLY/DAILY/WEEKLY/MONTHLY/YEARLY reachability checks account for frequency-aware candidate-set cardinality so permanently impossible positions terminate;
- BYSETPOS requires at least one supported BY selector, rejects zero/out-of-range/duplicate positions, ignores positions outside the current candidate-set size, and deduplicates alias positions that resolve to the same slot;
- BYSETPOS is shared by visible expansion and override-target validation, preserving original-slot identity and exception semantics;
- first-year candidates before DTSTART are omitted, impossible civil dates/missing fifth weekdays are skipped, and BYMONTH alone continues to preserve DTSTART's civil day where valid;
- deterministic occurrence UUID derived from the canonical event and original recurrence slot;
- RDATE additions and EXDATE exclusions stored inside the existing schema-v14 recurrence JSON;
- moved occurrence overrides that retain the original occurrence UUID/lineage while rendering at the replacement time;
- cancelled occurrence overrides that remain materialized and queryable with `cancelled` lifecycle status rather than disappearing;
- detached moved overrides are considered even when their original slot is outside the active horizon, so an instance moved into the visible window is not lost;
- expansion only inside the active query/view horizon rather than eager infinite materialization;
- recurrence candidates are retrieved independently from the base date-window query so a long-lived series can appear years after DTSTART;
- date-only, all-day, floating, and exact instant bases;
- preservation of all-day/range duration;
- exact/source-timezone recurrence by source wall clock across DST;
- invalid calendar dates in monthly/yearly series are skipped rather than coerced;
- recurrence definitions, SECONDLY/MINUTELY/HOURLY calendar/time limiters, daily-BYDAY/BYMONTH/BYMONTHDAY, monthly-BYMONTH/BYMONTHDAY, weekly/monthly/yearly-plain-BYDAY/WKST/BYWEEKNO/BYYEARDAY/yearly-BYMONTHDAY/monthly-ordinal-BYDAY/yearly-BYMONTH/BYHOUR/BYMINUTE/BYSECOND/BYSETPOS selector constraints, DATE-vs-date-time frequency/time-selector compatibility, exception time kinds/conflicts, and override target membership are validated at the SQLite persistence boundary; an override cannot manufacture a slot that does not exist in the RRULE/RDATE occurrence set;
- recurrence authoring mirrors those constraints contextually: selector affordances are frequency/time-kind aware, unavailable persisted values are preserved visibly rather than silently discarded, and live domain validation remains the final save gate;
- month/year/unresolved precision is rejected as a recurrence base instead of failing later during view materialization;
- materialized occurrences retain canonical event lineage, recurrence origin/index, original occurrence time, and override posture in the inspector.

Still ahead in this recurrence layer: remaining RFC edge semantics and selector combinations beyond the now-implemented seven-frequency BY-part matrix, explicit leap-second representation before `BYSECOND=60` can be accepted, richer high-level recurrence editing ergonomics, and completion of source-adapter interoperability beyond the verified RRULE + RDATE/EXDATE + RECURRENCE-ID codecs (full VEVENT transport remains ahead).

Implemented event metadata includes:

- canonical local UUID
- source UUID
- source-record identity
- Taria event ref
- Taria reconciled-event key
- assertion refs
- source refs
- provenance refs
- renderability
- normalized/raw title
- description
- event type
- domain
- jurisdiction
- institution
- lifecycle status
- confidence
- importance
- personal relevance
- tags
- extensible JSON properties

### RFC 5545 recurrence adapter

The new `ical` module provides a strict canonical RRULE boundary for future ICS/WebCal/CalDAV ingestion and export. It accepts bare RECUR values or `RRULE:` property values, maps every currently supported canonical recurrence selector, rejects duplicate or unknown rule parts, enforces COUNT/UNTIL exclusivity, and delegates selector range/context validation to the existing recurrence domain model. Because Ephemeris currently stores recurrence UNTIL as a civil date, RFC DATE-TIME UNTIL values are rejected explicitly rather than truncated. RDATE and EXDATE now share a strict canonical property adapter that reconstructs the master series' temporal kind, duration, and source-timezone shape while supporting RFC DATE, floating DATE-TIME, UTC DATE-TIME, and matching-TZID exact values. RDATE `VALUE=PERIOD` remains deliberately unsupported until occurrence-specific periods have canonical semantics. RECURRENCE-ID now has a strict original-slot codec for DATE, floating DATE-TIME, UTC DATE-TIME, and matching-TZID source-local DATE-TIME values. It preserves the canonical recurrence slot's temporal shape, rejects multiple values and incompatible DTSTART forms, and rejects `RANGE=THISANDFUTURE` until range overrides have canonical domain semantics instead of flattening them into a single-instance override. VEVENT DTSTART/DTEND transport is now also strict and canonical: RFC DATE maps only to explicit all-day events, floating/UTC/TZID DATE-TIME forms remain distinct, DTEND is validated as a matching non-inclusive/later endpoint, source-local TZID events preserve exact instants across DST, and imprecise `DateOnly` values are rejected on export rather than being strengthened into false all-day semantics. The RFC content-line/component-envelope layer is now implemented too: quote-aware parameter parsing, RFC TEXT escaping, CRLF unfolding, UTF-8-safe 75-octet folding, and a single-VEVENT envelope preserve unknown properties instead of silently discarding them. Typed VEVENT binding now covers UID, DTSTAMP, DTSTART/DTEND, SUMMARY, DESCRIPTION, STATUS, SEQUENCE, RRULE, RDATE, EXDATE, raw RECURRENCE-ID, and preserved unknown properties; unsupported DURATION is rejected rather than ignored. Master/detached assembly binds RECURRENCE-ID against the master's canonical original-slot shape, enforces UID identity, maps moved/cancelled/cancelled+moved temporal exceptions into canonical overrides, runs the finished rule through domain recurrence validation, rejects phantom targets, and refuses occurrence-specific semantics the current canonical override model cannot represent. Canonical projection is now implemented too: standalone VEVENTs and assembled master/detached series project into real `TemporalEvent` values with UID as source-record identity, SUMMARY/DESCRIPTION/standard STATUS mapped into canonical fields, recurrence validated through the domain model, and normalized master/detached VEVENT components retained under `properties.ical` instead of being discarded. Strict VCALENDAR ingestion is now implemented: VERSION 2.0 and PRODID are validated, supported top-level properties are preserved, VEVENTs are grouped by UID, exactly one master is required when detached instances exist, master/detached recurrence is assembled through the verified RECURRENCE-ID path, and supported groups project directly into canonical `TemporalEvent` values. Unsupported nested component families such as VTIMEZONE and VALARM are explicit errors rather than silently skipped. Canonical export is now implemented and verified too: supported `TemporalEvent` values serialize into deterministic VEVENT/VCALENDAR output, imported RFC UID and representable source properties are preserved when available, native events receive stable UUID URN UIDs, DTSTAMP is derived deterministically from canonical `updated_at`, recurrence exceptions become detached RECURRENCE-ID components, and round-trip tests cover moved/cancelled overrides plus preserved unknown master properties. Unsupported canonical statuses and imprecise `DateOnly` time are rejected rather than coerced. The next boundary is integrating the verified RFC transport with Ephemeris' source/store and user-facing import/export surfaces.

### Taria ingestion

Implemented against the real Resourcearium reconciled-event-set shape.

The importer supports:

- projection identity
- reconciled projection identity
- regular and top-level blocked events
- date precision
- date ranges
- month precision
- year precision
- offset date-times
- timezone/local date-times
- unresolved temporal values
- source contexts
- geography
- field-resolution/provenance payload retention

Import behavior:

- transactional per source artifact
- stable source-record identity
- creates new records
- updates changed records in place
- distinguishes unchanged records
- retains records missing from later snapshots rather than assuming deletion/cancellation
- reports blocked/unplaced and imprecise counts
- chooses a useful initial focus date after import

Entry surfaces:

- one-click **Update Taria Sources** against a configured local Resourcearium tree
- automatic sibling/local Taria checkout detection
- drag/drop reconciled JSON onto the GUI
- `ephemeris-import` CLI

### Taria bundle-release contract

Resourcearium now owns a consumer-facing `TemporalBundleRelease` layer over frozen derivative products.

The accepted Ephemeris contract is documented in:

- `docs/TARIA_BUNDLE_CONTRACT.md`

Current upstream state:

- 429 canonical temporal Resources
- 177 direct single-Resource RICS profiles in the first exact-lineage production tranche
- canonical 13-projection build tooling
- frozen release packaging and validation
- bootstrap channel currently pointing at `temporal-bundle-release:bootstrap:2026-10-04:r10`
- bootstrap posture: 1,357 unique ready events, 15 represented canonical Resources, 38 selected ingestion profiles, 196 recovered source surfaces, 8 partial canonical domain slots, 4 explicit gap-only slots, 0 pending slots
- production channel independently advances as live acquisition-backed releases become available

Important consumer rule:

- ReconciledProjectionEventSet carries event payload.
- CalendarSet carries membership/navigation metadata.
- overlapping bundle membership must never clone event identity.

The direct reconciled-event-set importer and CompactReconciledEventIndex importer are now low-level payload adapters beneath the local release updater.

Current bootstrap r10 update behavior:

- recovered U.S. Politics and U.S. Holidays are consumed through pinned `CompactReconciledEventIndex` payloads;
- European Elections and U.S. Sports are consumed through full `ReconciledProjectionEventSet` payloads;
- additional Economics, Finance, Business, Culture, and Education frozen-rebuild shards are consumed through compact reconciled indexes;
- Science/Technology/Space, Public Health, Environment/Weather, and Transportation/Civic Infrastructure are explicitly represented as gap-only rather than silently empty;
- all populated bootstrap shards expose accepted post-reconciliation payloads;
- CalendarSets and their event memberships are persisted independently from canonical event identity;
- bootstrap content-fingerprint semantics are validated exactly as Resourcearium defines them;
- local artifact paths remain confined to the configured Resourcearium tree.

Production support is also implemented:

- `bundle_artifacts[]` are resolved from the same filesystem updater;
- production file SHA-256 semantics are validated;
- upstream event/reconciled aliases provide global deduplication across overlapping projections;
- one canonical event can therefore carry several release/calendar memberships without cloning;
- the production overlap regression test proves one shared event across Politics and Finance remains one local event with two memberships.

The live Resourcearium production channel is still unset, so this production path is implemented and tested synthetically but not yet exercised against a live production release.

### Calendar presentation

Date-range modes:

- Year
- Quarter
- Month
- Week
- Day

Layouts are independent from date range:

- Grid
- Agenda
- Compact Agenda
- Stream
- Timeline
- Density
- Summary
- Table

That separation is deliberate. A Month view can be rendered through any of these presentation surfaces without redefining the temporal query or duplicating canonical events.

Stream has intentionally different ordering semantics from Agenda/Compact/Table: it is always chronological. Month/year precision markers sort before date-only/all-day records at their anchor date, timed instant/floating records then sort by actual local clock time, and unresolved records sort last. Saved grouping/sort settings remain preserved but inactive while Stream is selected.

Timeline is a proportional interval view over the active Year/Quarter/Month/Week/Day window. Exact/floating instants render as points; date-only/all-day/range records render as spans; month/year precision spans their real coarse interval; unresolved records are not assigned fake positions. The axis uses view-scale ticks (month, week/day, or six-hour ticks).

Density aggregates genuinely positioned events per day and renders relative intensity. Coarse month/year precision records are reported separately rather than sprayed across invented days. Clicking a Density day drills into Agenda + Day.

Summary is the first pivot/summary foundation. It reuses the existing GroupBy dimension as its pivot axis and reports group count, share, earliest/latest positioned date, plus a precision breakdown. Sort/color settings remain preserved but inactive for aggregate rows.

Agenda, Compact Agenda, and Table use true group partitioning: grouping remains independent from the active stable sort rules rather than merely emitting repeated headings whenever the sort order changes group values.

### Querying and presentation

Current query surface:

- text search
- domain
- jurisdiction
- event type
- institution
- renderability
- tag membership
- lifecycle status
- source visibility
- current-release Taria bundle membership
- current-release projected CalendarSet membership

Source visibility is independent from the event query.

Current presentation dimensions are also independent:

- grouping by date, week, month, source, domain, jurisdiction, institution, event type, or status
- stable multi-key sorting
- semantic fallback coloring by source, domain, jurisdiction, institution, event type, or status
- ordered query-driven color rules with first-match precedence
- Grid, Agenda, Compact Agenda, Stream, Timeline, Density, Summary, and Table layouts
- ordered visible Table-column configuration independent from filtering/grouping/sorting

These dimensions persist in saved views and do not reorganize or duplicate canonical events.

The advanced boolean/query-expression system is now implemented as a first working vertical slice.

Advanced queries support:

- arbitrarily nested AND / OR / NOT groups
- typed text predicates
- text set membership
- lifecycle-status sets
- integer comparisons for importance and personal relevance
- exists / missing predicates
- temporal-kind sets
- explicit civil-date overlap
- timezone-aware instant-to-date evaluation
- opt-in month/year imprecise-span matching
- relative civil-date windows anchored to the view timezone's current day
- Taria bundle-membership predicates by stable bundle ref
- Taria projected-calendar-membership predicates by stable calendar ID
- recursive GUI editing
- persistence through UI state and saved views
- backward-compatible loading of older saved query JSON

Simple facets remain convenient top-level filters and are ANDed with the advanced expression tree. Domain, jurisdiction, event type, institution, renderability, tag membership, and lifecycle status are all first-class saved facets. Their option sets are derived from the loaded canonical corpus, and older serialized queries/UI state remain compatible through serde defaults.

### Saved views

Named saved views are real durable product objects stored in SQLite.

A saved view currently retains:

- event query
- hidden/visible source selection
- date-range mode
- layout
- grouping
- stable multi-key sort rules
- semantic color fallback
- ordered color rules
- ordered composition layers
- overlays
- ordered visible Table columns
- display timezone
- week-start behavior

Saved views can be:

- created
- applied
- updated from current state
- deleted

They do not copy or own events.

Legacy saved-view data briefly stored in `ui-state.json` is migrated into SQLite on startup.

### Inspection

The event inspector exposes the normalized event plus rich Taria identity/provenance context. The recurrence authoring slice adds a canonical-series editor directly from this inspector for editable local/non-read-only events; materialized occurrences resolve back to their canonical event before editing.

Unplaced/conflicted events remain visible and inspectable rather than being dropped or assigned fake calendar dates.

## Completed inheritance work

The focused Rivetr calendar audit is complete.

Reused/adapted ideas:

- date navigation/math
- native calendar rendering patterns
- year/quarter/month/week/day behavior
- keyboard-first interaction
- local persisted UI state
- import reconciliation lessons

Explicitly rejected as canonical Ephemeris architecture:

- `TaskDto` as event
- task statuses as complete temporal lifecycle
- tags as the only ontology/provenance structure
- task datastore as temporal store
- unrelated Rivetr workspaces

See `IMPLEMENTATION_AUDIT_2026-10-04.md` and `RIVETR_INHERITANCE.md`.

## Important incomplete areas

The current milestone is a foundation, not the finished calendar.

Not yet implemented:

- richer per-field release diff presentation beyond canonical add/remove/time/status/cancellation changes
- general-purpose provenance/history tables beyond the Taria release snapshot model
- annotations
- relations and collections
- duplicate/entity resolution
- responsive shell/layout hardening for constrained widths and side-panel collapse
- reminders
- ICS/webcal/CalDAV ingestion/export
- full personal event editing

### Ordered color rules

Color is now a rule engine rather than a calendar-container property.

Implemented:

- rules reuse the full recursive query language, including temporal predicates
- rules are ordered and deterministic
- first enabled matching rule wins
- explicit RGB colors
- enable/disable, edit, add/delete, and reorder controls
- existing semantic ColorBy strategy remains the fallback
- rules apply consistently across Grid, Agenda, Compact Agenda, Stream, Timeline, Table, and unplaced records; aggregate Density/Summary views intentionally do not assign one event color to multi-event cells/rows
- rules persist in saved views

### Calendar algebra foundation

The ordered composition foundation is implemented and verified.

A saved view can persist sequential logical composition layers with these operators:

- union
- intersection
- subtraction

Evaluation is deterministic and ordered:

```text
base query
    -> composition layer 1
    -> composition layer 2
    -> ...
    -> overlay union
```

Composition layers are currently wired through:

- runtime event visibility
- transient UI state
- SavedView capture/apply
- SQLite persistence/migration

The composition model is intentionally event-native and does not copy membership.

The GUI editor is now implemented: layers can be enabled/disabled, named, assigned Union/Intersect/Subtract, reordered, deleted, and edited with the same recursive query editor used elsewhere.

Saved-view-reference composition is also implemented:

- each composition layer may use either its embedded query or a stable SavedView UUID as its operand;
- referenced views recursively contribute their query, hidden-source selection, composition layers, and overlays;
- presentation state is intentionally not inherited;
- renames do not break references because identity is UUID-based;
- missing/deleted references are explicit safe no-ops;
- direct and indirect cycles are rejected both by the application save/update path and at SQLite persistence;
- runtime traversal also guards against cyclic external/corrupt data by skipping the cyclic edge.

### Overlays

The first overlay slice is implemented.

An overlay is an independently defined query rendered into the same canonical event result without copying event membership.

Implemented:

- enabled overlays union their query matches with the base view query
- global source visibility remains independent
- overlay order is styling precedence
- overlays have independent semantic fallback coloring
- overlays can carry their own ordered color rules
- recursive overlay query editing
- overlay enable/disable, naming, reorder, and deletion
- overlays persist in saved views

## Immediate next implementation boundary

Phase 6 source management/history is now active on top of the completed release-adoption execution boundary.

Implemented in the current slice:

1. the Sources/Taria panel shows the adopted release ID, channel, status, completeness, generated/adopted times, coverage counts, and per-canonical-bundle population posture;
2. partial, pending, and gap-only states are rendered from the persisted manifest rather than hard-coded release assumptions;
3. bundle-membership predicates offer selectable refs from the adopted release;
4. projected-calendar predicates offer selectable stable calendar IDs with human-readable names/bundle context;
5. raw ref text remains editable for portability/debugging;
6. query choices come from SQLite, so normal view editing does not require live Taria filesystem access.

Whole-release transaction/rollback is now implemented and verified:

- one outer SQLite transaction encloses release metadata, all payload imports, upstream identity aliases, CalendarSets, projected calendars, and memberships;
- nested Taria import helpers join the existing transaction rather than committing independently;
- if any later shard/artifact fails, all earlier mutations from that release are rolled back;
- the rollback regression deliberately imports one valid shard, fails a later shard integrity check, and verifies zero events, zero release metadata, and zero memberships remain.

Background execution is now implemented and verified:

- **Update Taria Sources** starts a worker thread;
- the worker opens its own SQLite connection to the same WAL database;
- release adoption remains one atomic transaction in that worker connection;
- Taria path/channel controls are disabled while the update runs;
- both update buttons show a running state;
- egui polls the worker without blocking and reloads the calendar only after completion.

Release-history/diff inspection is now implemented from persisted SQLite state:

- adopted releases are listed without reopening Taria files;
- current release is compared with the previous release on the same channel;
- schema v11 persists the exact source projections associated with each adopted release;
- source rollover diffs report added/removed projection refs;
- schema v12 stores one canonical event snapshot per event actually present in each release, after cross-projection reconciliation;
- retained-missing local records are not falsely counted as present in later release snapshots;
- canonical snapshot diffs report added/removed events, title changes, temporal moves, lifecycle-status changes, and newly cancelled events;
- expandable change details expose event title/UUID plus before/after title, status, and temporal representation;
- resolved CalendarSet member-event deltas remain a separate membership-level view;
- snapshot capture participates in the same whole-release transaction, including rollback;
- older pre-v12 releases are not fabricated/backfilled; re-adoption can populate snapshots from the immutable release payload.

Refresh-attempt history and health are also persisted:

- schema v13 adds generic source-refresh attempt records;
- a Taria refresh attempt is written before the background worker starts;
- completed attempts retain success/failure, completion time, release ID, summary, and error;
- an app crash/restart can therefore leave an explicit incomplete attempt rather than erasing the attempt;
- the UI distinguishes running from orphaned/incomplete attempts;
- Taria refresh health is explicit: never-refreshed, running, healthy, stale, failed, interrupted, or unknown;
- upgraded pre-v13 databases with an adopted release but no attempt history are reported as unknown/legacy rather than falsely "never refreshed";
- the current local Taria policy marks a successful refresh stale after 7 days without another successful refresh, and the threshold is shown in the UI.

Calendar algebra is also now editable in the GUI.

### Source inspector and release posture

The Sources panel now has a first Phase 6 source-management surface.

Implemented:

- source selection independent from visibility toggles;
- source identity, external ref, publisher, kind, authority, locator, enabled/read-only state, creation time, local refresh time, and raw properties inspection;
- upstream Taria generation time when present;
- canonical event counts that include both direct ownership and source/import-record mappings, so reconciled cross-source events count for every contributing source;
- persisted release-to-source projection membership;
- current-release versus historical/not-current source posture;
- neutral handling for upgraded databases that have not yet recorded release/source links;
- source projection add/remove rollover in release-history diffs.

### Configurable Table columns

The dense Table layout now has a first-class ordered visible-column schema.

Implemented:

- 16 available columns spanning temporal display, core event facets, quality/renderability, tags, source, and upstream identity;
- add/hide/reorder/reset controls in the presentation editor;
- at least one visible column is retained by the editor;
- Table rendering is driven by the configured order rather than a hard-coded schema;
- transient UI-state persistence with backward-compatible defaults;
- SavedView capture/apply persistence;
- SQLite saved-view persistence through schema v10 and the v9 -> v10 migration.

The programmable-view inheritance/composition slice is complete. Phase 6 now has source inspection, release/source posture, canonical snapshot history, detailed release-event drill-down, persistent refresh-attempt history, and an explicit Taria health policy. The remaining Phase 6 boundary is broader non-Taria source refresh/history integration. The severe constrained-width UI/layout failure remains explicitly deferred layout debt.

## Rule going forward

Do not regress to a container-centric calendar.

The same canonical event corpus must remain independently:

- filterable
- groupable
- sortable
- colorable
- renderable in multiple layouts
- reusable by unlimited saved views
