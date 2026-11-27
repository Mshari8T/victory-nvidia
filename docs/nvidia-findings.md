# victory-nvidia: what was measured (2026-11-27)

One PC: Ryzen 5 5500 (6 cores, 12 threads, Zen 3 "Cezanne", PCIe 3.0 only), 32 GB,
RTX 5060 Ti 16 GB on PCIe 3.0 x8 (ASUS ROG STRIX B550-F), Windows 10 22H2 with the
D3D12 Agility SDK 1.620 in `target\diag\D3D12`, driver 610.74. Diagnostics build.
Scene: the prologue cutscene from a fresh start (the prison window shot), about two
minutes after the start screen, the same in every run. Figures are the steady
per-5-second lines; read differences under about 0.5 fps as noise.

## Frame rate

| Run | Console | fps | ms/frame | GPU busy |
|---|---|---|---|---|
| upstream `play.ps1` | Series X | 6.5 | 155 | 94-99% |
| + renderer ports 6, 7 (journal, guard) | Series X | 6.5 | 155 | |
| + Series S | Series S | 9.1 | 110 | 81% |
| + card buffers from 128 KiB, 1.5 GB | Series S | 9.8 | 102 | 73% |
| + 4 list-call threads (`VICTORY_THIN_THREADS=4`) | Series S | **10.6** | 94 | |
| + under/view threads 3 | Series S | 10.5 | 95 | |
| + `VICTORY_ASK_EARLY=2` | Series S | 10.4 | 96 | |
| the profile, Series X | Series X | 7.4 | 133 | 99% |
| the profile, Series X, `5cf00dc39042edaf` skipped | Series X | 8.1 | 123 | 86% |

The profile is the default of `scripts\play.ps1` now (`-Stock` for upstream's).

## Where a frame goes

- **Series X: the card.** 99% busy. A timing run (`VICTORY_GPU_TIMING=1
  VICTORY_D3D12_DRED=1`) puts the title's own commands at 87 ms a frame (dispatch
  43.5, indirect 32.0, mesh 6.2, alias copies 2.9, draws 2.0); the rest of the 151 ms
  is our passes. One compute dispatch, `5cf00dc39042edaf`, is 12.35 ms of it alone
  (14%). Its DXIL: WaveSize 32, 35 `SampleCmpLevelZero`, 62 `SampleLevel`, 394
  branches, 321 phis, lane ids rebuilt from `WaveReadLaneFirst(tid) & -32`: a
  shadow/lighting resolve. Skipping it leaves tile garbage along the bottom edge,
  so it must be made faster, not dropped. Next: 0a5ecfee6b79e13b 2.7 ms,
  cb6adcc6da2613ff 2.1, 34fb51b8a0312e82 1.8, mesh db875f50a166bd91 1.8 (360 calls).
- **Series S: the submitting thread.** It works 95 ms a frame and never waits for the
  card: lists' steps 39 (refreshes 25, of 8,400 flushes 7,400 copy nothing), copies
  in the card's memory 16-20, the write watch 10, the title's bytes under flights 9,
  planning card steps 6-9. The title's threads wait 37 ms at presents for it.
- **The CPU is full**: 89% over all 12 threads. Eight list-call threads fought the
  title's own; four were faster.
- **The bus**: 85-120 MB sent up and 45-70 MB read back a frame over PCIe 3.0 x8.

## Faces

Black heads (the window shot) were gone with the profile in every run since. Not
isolated: the profile changes the fetch check, thread priorities (`TITLE_PRIORITIES`
left unset) and ports 6/7 together. The fetch check caught 0 torn pieces in the one
run it was read, so it is likely not the race; late textures with starved streaming
threads fit better.

## Next

1. `5cf00dc39042edaf` and the other heavy dispatches: why they cost so much on a
   32-lane card (the wave64-as-two-halves lowering, waterfall loops, loop limits:
   `VICTORY_SHADER_LOOP_LIMIT` as a probe).
2. The submitting thread: refreshes that copy nothing still cost 25 ms; split the
   preparing over two threads (docs/findings.md, item 1).
3. Host FSR from a Series S render, as the AMD forks do (`VICTORY_FSR_PC`, not in
   this tree yet).
