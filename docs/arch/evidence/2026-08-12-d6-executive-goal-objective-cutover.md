# D6 Executive Goal objective-model cutover

- Objective rows/status/summary are written and read only by Executive's Goal
  SQLite application store; they now live in `application::goal::objective`.
- GoalService imports the owner model directly.
- Fabric removed three public/census rows and the old module without an alias.
