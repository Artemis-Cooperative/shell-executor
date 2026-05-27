# Context

Domain vocabulary for shell-executor. Architecture reviews and refactor proposals should use these terms exactly.

## Terms

**Outcome**
The completion-only record of one command's execution. Carries `status`, `output` (see *OutputCapture*), `elapsed`, `label`, `signal_num`, and `timed_out`. Built once, at the point a command finishes. Consumed by rendering, logging, status aggregation, and exit-code derivation.

Outcome does not model a running command. Live state stays local to whoever is rendering it (see *Slot*).

**OutputCapture**
How a command's output was obtained. Two variants:
- `Captured(CommandOutput)` — stdout/stderr were captured and are available for logging and the success closure. Used by the default single-command path, parallel children, and the TUI (which captures the PTY buffer; stderr is empty by construction).
- `Inherited` — stdout/stderr were connected directly to the parent terminal; nothing was captured. Used by succinct mode and interactive PTY mode.

The variant determines whether a body can be included in logs or evaluated by a success predicate.

**Slot**
Per-child live state used while a parallel group is still spinning. `Running` until the child exits, then `Done(Outcome)`. Private to the modules that drive in-place rendering (`parallel.rs`, `tui.rs`). Not part of the library surface.
