# Process supervisor

Job-scoped child-process ownership for production adapters.

On Windows, commands are created suspended, assigned to a private Job Object
with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and resumed only after assignment.
Stopping or dropping the handle terminates the complete associated process tree.
The module exposes no shell command construction and owns no opencode, broker,
SRT, Tauri, or document policy.
