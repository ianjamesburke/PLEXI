Always confirm best practices by researching the docs.

## Lessons

- **Coupled state:** When adding new state that derives from or shadows existing state (e.g., `zoomed_pane` tracking `focused_pane`), grep for all mutation sites of the original state and update each one to handle the new state.
