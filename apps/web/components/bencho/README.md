# Bencho blocks

These blocks come from the free half of [Bencho](https://bencho.dev)
([source](https://github.com/cabralorenzo/bencho)) by Lorenzo Cabra, under the MIT
license in `LICENSE` here. The neutrals in `app/globals.css` come from the same
project's `tokens.css`.

They are adapted, so an update is a merge by hand. What changed:

- `framer-motion` imports became `motion/react` (the same library, already a
  dependency) and `lucide-react` icons became inline SVG, so no package was added.
- `stacked-bars`: takes real series and days instead of generating traffic, fills
  its container, and shows conflicts under the baseline. The Day/Week switch is gone.
- `toast-stack`: the messages are props.
- `time-scrubber`: moves through a list of operations instead of the hours of a day.
- `dynamic-island`: one activity, who is working now. Timer, call, music and flight
  are gone, and it opens over the page instead of pushing it.

The Bencho name and goat mark are not covered by the license and are not used. Blocks
behind Bencho Pro are paid and are not in this repository.
