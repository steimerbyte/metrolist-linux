# audio — frozen source

## Source

| file | sha256 (at freeze) |
|------|--------------------|
| crates/audio/src/controller.rs | 82ab38dc48e24ebf3d4a9bb381ef884796afeeb630862258b2e51263b09eed18 |
| crates/audio/src/engine.rs | 61eeaa0699c897000df1244e555c0090e0aa2ce8ac132272a27e19ce470b4dca |

Toolchain: Verus `0.2026.11.02.8bf68ee` (tag `release/0.2026.11.02.8bf68ee`),
bundled Rust `1.99.1-x86_64-unknown-linux-gnu`; workspace pin `1.99.1`.
Lynette gate: `ed5855fb68c18a77a5ec0c98512497c5ef4e7d59`.

## Scope

The clock-free arithmetic obligations frozen by audio2:

- `PlaybackController::set_sleep_timer` — `SleepTimer::Minutes(m)` becomes
  `(m as u64 * 60).min(24 * 60 * 60)` seconds. `Instant::now() + Duration` is
  out of subset; only the seconds value is proven.
- `Ipc::request` — `self.next_id.wrapping_add(1)` is the id sent with
  each IPC request.

## Model deviations

- The seconds computation is a free function `sleep_seconds(minutes: u32)`
  rather than a method, because the surrounding `Instant`/`Duration` code and
  the `sleep_deadline` field are not Verus values. The expression is verbatim.
- `u64` `min` is kept as `u64`; the spec compares in `int` (no wrap:
  `u32::MAX * 60 < u64::MAX`).
- `wrapping_add(1)` is modelled as `(id + 1) mod 2^64`; the exec call is the
  real `u64::wrapping_add`.
- `sleep_window_contains` is transcribed verbatim as a free function; it has
  no `self`, so nothing is modelled away for it.
- The rest of `controller.rs`/`engine.rs` (mpv subprocess, sockets,
  `serde_json`, `Instant`) is out of subset.

## Proven

- `sleep_seconds`: equals the spec; `lemma_sleep_clamped_to_a_day`,
  `lemma_sleep_exact_within_a_day` (exact for the 1..=1440 min range the UI
  offers), `lemma_sleep_monotone`.
- `next_request_id`: equals the spec; `lemma_ids_never_repeat` (consecutive
  ids always differ) and `lemma_id_increments_below_max`.
- `sleep_window_contains` (NEW in the 2026-11-02 re-freeze, real
  `controller.rs` fn, body verbatim including `% DAY` / `const DAY: u32`):
  equals `window_contains_spec` (int modulo, wrapped/ordered/full-day
  branches); `lemma_full_day_always_contains`,
  `lemma_ui_windows` (the controller's six unit-test cases) and
  `lemma_ordered_step` (later minute stays inside an ordered window).

## Re-freeze 2026-11-02 (parity-audio: crossfade / sleep-fade / normalization)

Reason: `crates/audio/src/{controller,engine}.rs` changed while parity-audio
landed crossfade, sleep-fade and normalization work; `audio2` verified the new
tree (`cargo test -p ml-audio` 51 unit + 11 live green, clippy clean).

The two previously proven obligations were re-read at the new revision and
keep their exact shape:

- `set_sleep_timer`: the `SleepTimer::Minutes` arm still computes
  `let seconds = (minutes as u64 * 60).min(24 * 60 * 60);` and turns it into
  `Some(Instant::now() + Duration::from_secs(seconds))`. What changed is
  surrounding behaviour: `clear_sleep_fade()` runs first and the new
  `EndOfTrack`/`FadeOut` arms were added (all out of subset).
- `Ipc::request`: untouched — `self.next_id = self.next_id.wrapping_add(1);`,
  then `let request_id = self.next_id;`, and `request_id` is the IPC id.

### New measured obligation

`sleep_window_contains(now, start, end)` is a new public, total, `u32`-only
function in `controller.rs` (the daily sleep-timer window; `start == end` is
the whole day, `start > end` wraps overnight). Its exec body is transcribed
verbatim into `original.rs`/`candidate.rs` (including `const DAY: u32 = 24 * 60`
and `% DAY`), with `window_contains_spec` as the readable int-modulo spec, and
these proof helpers in `candidate.rs` (all ghost, so the exec surface is
unchanged):

- `lemma_full_day_always_contains` — `start == end` contains every `now`.
- `lemma_ui_windows` — the controller's own six unit-test cases
  (22:30/05:00/12:00 in 22:00..06:00, 09:00/17:00 in 09:00..17:00,
  23:00..23:00 whole day).
- `lemma_ordered_step` — inside an ordered (non-wrapping) window a later
  minute is still inside.

`candidate.rs` now verifies 12 functions; `original.rs` (which carries no
lemmas) verifies 4.

### Out of subset at the new revision (unproven)

`same_album`, `crossfade_candidate`, `peek_next_song`, `tick_crossfade`,
`tick_sleep`, `handle_end_file`, `clamp_speed` (f32),
`resolve_muted`/`env_test_mute` (env IO), `initial_device` (subprocess IO),
`base_args` (string building), the `EngineState` `skip_silence` flag, and the
`FadeOut` volume ramp. `eq.rs` and `normalize.rs` are not frozen sources.

## Re-freeze 2026-11-02 16:13 (UI queue / crossfade integration)

Attribution: the `controller.rs` edit is ui-lead's queue/radio work (additive
`restore_queue`/`append_queue`/`move_queue_item`/`remove_queue_item` plus four
controller tests, `cargo test -p ml-audio` 55 passed + 11 live ignored, clippy
clean). The `engine.rs` helper changes at the same minute were later resolved as the
shutdown/leak work now recorded below (see the final re-freeze); they never
touched the proven `Ipc::request` obligation, which was re-read verbatim, and
the same green suite compiles and exercises the file.

Reason: both files changed again at 15:56, after the 15:19 re-freeze, while the
controller queue/crossfade API used by the UI landed. The three already proven
obligations were re-read at the new revision and keep their exact shape, so
`original.rs`/`candidate.rs` and their 12 proofs are unchanged — this is a
hash + semantic-delta record:

- `sleep_window_contains` — body unchanged at lines 61-73 (`% DAY`,
  `start == end` whole day, wrapped/ordered branches).
- `set_sleep_timer` — the `SleepTimer::Minutes` arm still computes
  `let seconds = (minutes as u64 * 60).min(24 * 60 * 60);` (line 521) and
  `Some(Instant::now() + Duration::from_secs(seconds))`.
- `Ipc::request` — still `self.next_id = self.next_id.wrapping_add(1);`
  (line 619), `let request_id = self.next_id;` (line 620), and that id is the
  IPC request id.

### New in this revision (not proven)

- `PlaybackController::play_queue` keeps the proven shape
  `start_index.min(queue.len() - 1)` but its controller state machine is
  still CANDIDATE.
- `restore_queue` — field copy plus UI emits; CANDIDATE.
- `append_queue` — dedup by `Song.id` through `HashSet<String>`, appends only
  new ids and returns the count; CANDIDATE (needs the `Queue` model plus a
  `HashSet` model; the same fold over `pages::next::append` is VERIFIED in
  `proofs/pages`).
- `move_queue_item` / `remove_queue_item` — delegate to the proven `Queue`
  `move_item`/`remove` then update controller state; CANDIDATE.
- `attach_crossfade_engine`, `set_crossfade` (clamped by
  `duration.min(MAX_CROSSFADE)`, 30 s), `set_crossfade_gapless`, `crossfade`,
  `is_crossfading`, `set_device`, `set_muted`, `set_skip_silence`,
  `set_speed`, `set_normalization`, `set_eq`, `stop`, `submit_resolved`,
  `poll`, `snapshot`, `queue_view` — channel/`Instant`/`f32` plumbing;
  OUT-OF-SUBSET, except `set_crossfade`'s pure `min` clamp, which is a
  candidate for the same treatment as `sleep_seconds`.
- engine: `resolve_binary`, `list_devices*`, `initial_device`,
  `env_test_mute`, `unique_socket_path`, `wait_for_socket`, `start`,
  `observe_all`, `set_device`, `apply_af`, `tick`, `teardown`, `kill`,
  `parse_audio_params` — subprocess/sockets/env/`serde_json`;
  OUT-OF-SUBSET.
- engine shutdown/leak fix (final re-freeze below): `runtime_dir`,
  `socket_owner_pid` (its own unit test only), `pid_alive` (`libc::kill`),
  `sweep_stale_sockets`/`sweep_stale_sockets_in` (`std::fs` + uid check),
  `die_with_parent` (`pre_exec`/`prctl`), `MpvEngine::pid`/`socket_path`/
  `shutdown`, `Ipc::take_child`, `PlaybackController::shutdown` — IO,
  `unsafe` libc and process control; OUT-OF-SUBSET. `socket_owner_pid` is a
  total `&str` parser and would be provable in a `bytes`/`char` model, but it
  is pinned by its own eleven-case unit test instead.

## Re-freeze 2026-11-02 16:31 (controller event/API addition)

Reason: `crates/audio/src/controller.rs` changed again at 16:22:33 (now
`26c72c24e2db4ed5ccdb8e3aaedcfaf28dbfb3014eace65b1acd9c2064a620e5`);
`engine.rs` is unchanged (`1f1944f7…`). The delta is +6 lines in the
controller's UI-facing event/API surface ahead of `play_queue` (the proven
calc bodies are untouched). All three proven obligations were re-read against
the current file and are verbatim, so `original.rs`/`candidate.rs` and the 12
proofs are unchanged — hash + delta update only, no re-prove:

- `sleep_window_contains` — lines 61-73 identical to the transcription
  (`% DAY`, `start == end` whole day, wrapped/ordered branches).
- `set_sleep_timer` — the `SleepTimer::Minutes` arm still computes
  `let seconds = (minutes as u64 * 60).min(24 * 60 * 60);` (line 527) and
  `Some(Instant::now() + Duration::from_secs(seconds))`.
- `Ipc::request` — `engine.rs` is byte-identical to the previous freeze,
  so `self.next_id = self.next_id.wrapping_add(1);` and the incremented IPC id
  are unchanged.

Attribution: ui-lead (confirmed this turn): the +6 lines are
`ControllerEvent::DeviceChanged { id, fallback_from }` plus one forwarding arm
from `AudioEvent::DeviceChanged`, added for the UI's device-fallback
notification. No proven body was touched. Test status at these hashes:
ml-audio lib 56 passed, ml-ui 33 lib + 4 integration passed, live crossfade
5 s passed, fmt clean.

## Re-freeze 2026-11-02 (shutdown/leak fix)

Reason: the app could leave `mpv` and both IPC sockets alive after
`SIGTERM`/`SIGKILL`, so `crates/audio/src/{engine,controller}.rs` changed
while the signal/shutdown work landed (new hashes in the table above:
controller `82ab38dc…`, engine `61eeaa06…`). The `engine.rs` helper delta that
the earlier note left unattributed is **this** change: `die_with_parent`
(`PR_SET_PDEATHSIG` + parent re-check), the shared `ChildSlot` so
`MpvEngine::shutdown` can reap the child even if the thread is wedged, the
sweep of dead-owner sockets, and the idempotent `shutdown`/`Drop` path; the
controller delta is `PlaybackController::shutdown`.

All three proven obligations were re-read against the current files and are
verbatim, so `original.rs`/`candidate.rs` and the 12 proofs are unchanged —
hash + delta update only, no re-prove:

- `sleep_window_contains` — verbatim, lines 61-73 of `controller.rs`
  (`% DAY`, `start == end` whole day, wrapped/ordered branches).
- `set_sleep_timer` — the `SleepTimer::Minutes` arm still computes
  `let seconds = (minutes as u64 * 60).min(24 * 60 * 60);` (line 527) and
  `Some(Instant::now() + Duration::from_secs(seconds))`.
- `Ipc::request` — still `self.next_id = self.next_id.wrapping_add(1);`
  (line 761), `let request_id = self.next_id;` (line 762), and that id is the
  IPC request id.

Not a proof-relevant change; the new lifecycle code is listed as
OUT-OF-SUBSET above. Test status at these hashes (workspace-wide, this turn):
`cargo test --workspace --no-fail-fast` 702 passed / 0 failed / 30 ignored,
`cargo fmt` clean, clippy no new warnings, and the debug e2e shows 0 `mpv`
processes and 0 sockets after smoke/screenshot/`SIGTERM` (a `SIGKILL` leaves
only stale sockets, which the next launch sweeps).
