# Roadmap

Where Hito is and what comes next. Only what is under "Done" exists; everything else is a plan.

## Done: first release (0.1.0)

- Server: accounts, sessions, workspaces with roles and invitations, guests limited to their
  projects, projects with milestones, tasks (status, priority, assignee, milestone, due date,
  labels, checklist, description), comments, an activity log, my tasks across projects, real time
  over an authenticated WebSocket, limits. SQLite with migrations.
- End-to-end encryption of task text, comments, milestone names and project descriptions with MLS
  (RFC 9420) through OpenMLS: one group per project, project keys shared inside it, a new key when
  someone leaves. Tests against a real server check what the database holds.
- Integrations: personal API tokens and signed webhooks, for Ágora and anything that accepts a signed POST of a project's
  changes in a channel once connected.
- Desktop app for Linux, macOS and Windows: overview, my tasks, board with drag and drop, list,
  milestones, activity, the task panel, project templates, the Enterprise suite's shell, a Home page, light,
  dark and high-contrast themes, Spanish and English, the global menu over D-Bus on Linux.
- Packages for every release: AppImage, .deb, .rpm and a tarball for Linux, a universal DMG with
  its own install window and a server zip for macOS, a portable zip for Windows, and the Linux
  server on its own (tarball with `install.sh`, `.deb`, `.rpm`). Merging a version bump with its
  notes releases it.

## Next

- A project's devices with their fingerprints in the app, so a device the server added shows.
- A Gantt timeline of tasks and milestones.
- File attachments, encrypted like the rest, with size limits.
- Recurring tasks, task dependencies, time estimates.
- Notifications on macOS and Windows; an in-app updater.
- Account settings: change password, sign out other sessions from the app, delete the account.
- Workspace administration in the app: roles, removing people.

## Later

- A web client from the same Rust code (WebAssembly), and mobile apps.
- PostgreSQL as a second storage backend.
- Two-factor sign-in, single sign-on.
- Verified identities for end-to-end encryption, and an independent review of how Hito uses MLS.
