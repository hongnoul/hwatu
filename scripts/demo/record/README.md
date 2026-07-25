# Automated demo studio

Records the README demo with no keyboard input and no windows on the real desktop. A nested headless Sway compositor runs an isolated hwatu daemon and `wf-recorder`.

The published hero is deliberately plain product footage: a real terminal,
real CLI output, and the same real WebKit sessions materializing for a human.
There are no slides, captions, synthetic browser frames, or presentation-only
animation.

## One command

```sh
scripts/demo/record/run-real.sh
```

This records one uninterrupted workflow, renders the MP4 and animated WebP,
builds an 800px contact sheet, and runs the local media checks.

After reviewing the contact sheet, publish and verify the GitHub README with:

```sh
scripts/demo/record/run-real.sh --publish
```

The publisher checks both asset URLs, GitHub's rendered README API, and the live GitHub DOM before it reports success.

## What the recording shows

1. Open the reference and app as real headless sessions.
2. Compare their rendered pixels and print the real score.
3. Focus that same headless app and scroll the live browser through the CLI.

## Components

```text
film-real.sh     uninterrupted terminal and browser recording
render-real.sh   MP4/WebP/contact-sheet rendering without compositing
validate-real.sh media decode, dimensions, duration, and animation checks
publish.sh      release upload, README update, and live verification
stage.sh        isolated compositor lifecycle
```

The previous composed v2 studio remains in this directory for reproducibility,
but it is not the README workflow.
