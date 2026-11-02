# Finding code by what it does

Status: **proposal, not built.** Measured on 2026-11-02.

The idea: `hb search "payment function"` finds the code that handles payments,
even if no line says "payment". The goal is a search that works for people and
saves agents from reading whole directories.

## What the evidence says

| Source | Finding |
|---|---|
| [Cursor](https://cursor.com/blog/semsearch) (their own numbers) | Semantic search on top of grep: 12.5% better answers on average, 2.6% better code retention on large repositories. They keep both. |
| [Sourcegraph](https://sourcegraph.com/blog/how-cody-understands-your-codebase) (2024) | Dropped embeddings for chat, for BM25 plus other signals. Reasons: code sent to a third party, indexes to maintain, cost at scale. No numbers given. |
| [Anthropic](https://www.anthropic.com/news/contextual-retrieval) (2024) | Embeddings + BM25 beat embeddings alone. Failure rate 5.7% to 2.9% with contextual chunks, 1.9% with reranking. Across several kinds of documents, not code alone. |

So nobody serious replaces grep. The question is what to add to it, and at
what cost.

## What we measured

`scripts/search-eval.py`, on Hebra's own Rust code: 1,533 chunks (one per
function or type, with its doc comment and path), 22 questions with a known
answer. Model: `nomic-embed-text`, local, through Ollama. "Top 5" means the right
file was among the first five results.

| Questions | Method | Top 1 | Top 5 | Top 10 |
|---|---|---:|---:|---:|
| English | Keywords (BM25, identifiers split) | 41% | 68% | 73% |
| English | `nomic-embed-text` | 59% | 73% | 77% |
| English | `nomic-embed-text` + keywords (RRF) | 59% | 68% | 73% |
| English | `mxbai-embed-large` | 45% | 68% | 73% |
| English | `mxbai-embed-large` + keywords (RRF) | 50% | 73% | 73% |
| 8 of 22 in Spanish | Keywords | 32% | 41% | 45% |
| 8 of 22 in Spanish | `nomic-embed-text` | 41% | 59% | 64% |
| 8 of 22 in Spanish | `nomic-embed-text` + keywords (RRF) | 36% | 45% | 50% |
| 8 of 22 in Spanish | `mxbai-embed-large` | 36% | 55% | 55% |

`mxbai-embed-large` only read the first 800 characters of each chunk: the Ollama endpoint rejected longer ones.

Twenty-two questions on one repository is a small sample: these numbers show
direction, not a benchmark. Three things stand out anyway.

1. **Keywords alone are already close.** With English questions, the top 5 goes
   from 68% to 73% at best. What embeddings improve is the first result, and
   only with one of the two models: 59% against 41%.
2. **The language of the question matters more than the technique.** The same
   questions in Spanish lose 14 points with embeddings and 27 with keywords.
   "función de pagos" finds nothing with any method; "payment function" works.
   Neither model bridges languages.
3. **Merging is not free.** Plain reciprocal rank fusion helped one model and
   hurt the other, because keywords add noise on the questions they cannot
   answer. It needs tuning before it earns its place.

Cost, on this machine:

| | |
|---|---|
| Models on disk | 274 MB and 669 MB |
| Indexing 1,533 chunks | 13 to 39 s (40 to 119 chunks/s) |
| A search over those vectors, plain scan | 0.4 ms |
| Index size, 32-bit floats | 4.6 MB |
| Index size, 8-bit integers, same top 5 | 1.1 MB |

A plain scan scales to about 100,000 chunks in tens of milliseconds, so no
vector database is needed. `sqlite-vec` says itself it is pre-v1, and nothing
here requires it.

## Recommendation

Build it in two steps, keyword search first.

**Step 1: `hb search`, always on, no model.** BM25 over chunks from tree-sitter
(Hebra already parses six languages for merging), in SQLite's FTS5, which is
already in the binary. No new dependency and nothing to download. It also
indexes what only Hebra has: change descriptions, tasks and the prompts behind
each change, so a question can match why code exists and not only what it says.
Works today for agents, who ask in the language of the code.

**Step 2: `hb search --semantic`, opt-in.** Embeddings from a model the user
already has (Ollama, or any compatible endpoint), never inside `hb`: the binary
stays small and no code leaves the machine unless someone configures that.

What makes Hebra a good home for this is that **chunks are content-addressed**.
A vector is stored under the hash of its chunk, not of a file or a commit. A
hundred agents in a hundred workspaces share one index, a branch only embeds
what it changed, and a commit touching one function embeds one function. Git-based
tools have to work this out from outside.

Vectors live in their own file (`search.db`), as 8-bit integers. It can always
be rebuilt, so it stays out of history and out of `hb push`. The server keeps its
own, so the hub can offer the same search.

## Before building step 2

- Measure a multilingual model. `potion-multilingual-128M` (model2vec) and
  similar ones are candidates; the point is whether "función de pagos" starts
  working. Measure on more repositories, with more questions.
- Tune the merge, or drop it: weighted scores, or keywords only when the query
  contains identifiers.
- Decide how the index updates without slowing commits. The likely answer is
  in the background, like `hb gc --auto`.

Without a multilingual model, a Spanish speaker gets good results only when
someone translates the question. Agents already do; the CLI does not.
