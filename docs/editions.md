# Consumer editions: what fits on which drive (measured 2026-11-05)

All sizes are measured from CI builds unless marked *estimate*. "On drive" is installed size;
the drive also needs ~10% free for updates and filesystem overhead.

## Components

| Component | On drive | Notes |
| --- | --- | --- |
| Desktop app + `kb` tool | ~11 MB per OS | macOS arm64, Windows x64, Linux x64 builds |
| llama.cpp b11747 | 16 MB (macOS), 36 MB (Windows Vulkan) + 23 MB (Windows CPU) | CUDA builds excluded (150–600 MB) |
| Qwen3 4B (Q4_K_M) | 2.5 GB | Apache-2.0; quality vs 8B pending model-eval |
| Qwen3 8B (Q4_K_M) | ~5.0 GB | Apache-2.0 |
| Medical & survival library | 31 MB | 13 U.S. government sources, ~47,000 passages; eval 50/50 |
| Wikipedia, 50,000 most-read articles | 1.5 GB | 1.73 M passages, CC BY-SA, "reference" authority |
| Texas map tiles + places | ~650 MB | street-level zoom 14, 27,657 places |
| Texas routing | ~950 MB *pre-compression measurement* | 1.3 GB RAM while navigating; unloads after 10 idle min |

## Editions

| Edition | Contents | On drive | Smallest drive |
| --- | --- | --- | --- |
| Essential (4B) | app, engine (Mac+Win), 4B, medical | ~2.6 GB | 8 GB |
| Essential (8B) | app, engine (Mac+Win), 8B, medical | ~5.1 GB | 8 GB |
| Essential + Texas | Essential (8B) + Texas map, places, routing | ~6.7 GB | 16 GB |
| Family | Essential + Texas (8B) + Wikipedia | ~8.2 GB | 16 GB |
| Family, both models | Family + 4B as fallback for 8 GB laptops | ~10.7 GB | 16 GB |

## What actually limits the cheap edition

Storage is no longer the constraint: every consumer edition fits on a 16 GB drive. The constraint is the
customer's computer: RAM for the model (≈ model size + 1–2 GB) plus 1.3 GB while routing. Machines below
the model's RAM floor run library search, maps and routing without generated answers.

## Not yet measured

- Real speed and peak RAM of 4B and 8B on a customer Mac and Windows laptop (`kb bench`).
- Whether 4B answers are good enough (model-eval quality, then clinician review).
- Read speed of inexpensive USB flash drives (model load time).
