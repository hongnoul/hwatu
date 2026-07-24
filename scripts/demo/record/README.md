# Automated demo recording rig

Records hwatu demos with zero human keyboard time and zero windows on
the real desktop. The "studio" is a nested sway compositor on the
wlroots headless backend, with its own runtime dir, its own hwatu
daemon, a remote-controlled kitty terminal, and wf-recorder capturing
the virtual 1920x1080 output.

```
stage.sh   # studio lifecycle: up / rec / stoprec / shot / type / down
film.sh    # the shoot: runs every beat of the convergence demo,
           # emits raw mp4 + machine-readable beat markers
render.sh  # raw mp4 -> README webp loop + release mp4 (jcode pattern)
```

## One command

```sh
scripts/demo/record/film.sh /tmp/out/demo-raw.mp4
scripts/demo/record/render.sh /tmp/out/demo-raw.mp4
```

`film.sh` types each command into the staged terminal with a human
rhythm (tunable via HWATU_DEMO_TYPE_DELAY), so the recording looks
like a person driving, but every take is identical and repeatable.
Beat timestamps land in `demo-raw.marks` for cutting.

## The climb

Beat 4 of `film.sh` iterates over `scripts/demo/checkpoints/*/`:
serve each checkpoint dir (a progressively better clone), re-diff,
and the score climbs on camera. Drop convergence checkpoints from the
clone swarm in there, ordered by name (e.g. `01-87pct/`, `02-93pct/`,
`03-98pct/`).

## Verify a take without watching it

```sh
scripts/demo/record/stage.sh shot /tmp/frame.png   # while staged
ffprobe demo-raw.mp4                                # duration/res
cat demo-raw.marks                                  # beat timings
```
