"""Measures ways of finding code from a description, on Hebra's own source.

Chunks: one per function, struct, enum, trait or impl header, with its doc
comment and path. Compares BM25 over split identifiers, embeddings from a local
Ollama model, and both merged with reciprocal rank fusion. See
docs/design/semantic-search.md.

    ollama serve &
    python scripts/search-eval.py nomic-embed-text
"""
import json
import math
import re
import sys
import time
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent / "crates"
MODEL = sys.argv[1] if len(sys.argv) > 1 else "nomic-embed-text"
DOC_PREFIX = "search_document: " if "nomic" in MODEL else ""
QUERY_PREFIX = "search_query: " if "nomic" in MODEL else (
    "Represent this sentence for searching relevant passages: " if "mxbai" in MODEL else ""
)

# (query, file that answers it)
QUERIES = [
    ("función de pagos", "billing/stripe.rs"),
    ("cobrar a un cliente con tarjeta", "billing/stripe.rs"),
    ("verify the signature of a payment webhook", "billing/stripe.rs"),
    ("limit how many requests a client can make", "ratelimit.rs"),
    ("find who wrote each line of a file", "blame.rs"),
    ("detectar archivos renombrados", "rename.rs"),
    ("compress versions of a file against each other", "delta.rs"),
    ("cifrar los objetos en disco", "store.rs"),
    ("what happens when two processes publish at the same time", "oplog.rs"),
    ("sign in with github", "identity/github.rs"),
    ("give an agent a narrower credential", "mandate/src/lib.rs"),
    ("reintentar cuando la base de datos está ocupada", "oplog.rs"),
    ("convert windows line endings", "wc.rs"),
    ("find the bad review when a merge batch fails", "queue.rs"),
    ("quién está trabajando ahora mismo", "presence.rs"),
    ("tamper evident log of bookmark moves", "transparency.rs"),
    ("parse the stream git produces when exporting", "gitimport.rs"),
    ("reservar un archivo para que otro agente no lo toque", "claims.rs"),
    ("stop agents from changing certain files", "bookmarks.rs"),
    ("hash a session token before storing it", "identity/secret.rs"),
    ("binary search for the commit that broke the tests", "extras.rs"),
    ("guardar cambios sin terminar para luego", "history.rs"),
]

ITEM = re.compile(
    r"^( {0,4})(pub(\([a-z]+\))? )?(async )?(unsafe )?(fn|struct|enum|trait|impl|const|type)\b"
)


def chunks_of(path: Path):
    lines = path.read_text(encoding="utf8").splitlines()
    rel = str(path.relative_to(ROOT)).replace("\\", "/")
    out, i = [], 0
    while i < len(lines):
        if not ITEM.match(lines[i]):
            i += 1
            continue
        start = i
        while start > 0 and lines[start - 1].lstrip().startswith(("///", "#[")):
            start -= 1
        indent = len(lines[i]) - len(lines[i].lstrip())
        end = i + 1
        while end < len(lines) and end - i < 80:
            line = lines[end]
            if line.strip() and len(line) - len(line.lstrip()) <= indent and line.strip()[0] == "}":
                end += 1
                break
            if line.strip() and len(line) - len(line.lstrip()) <= indent and ITEM.match(line):
                break
            end += 1
        body = "\n".join(lines[start:end])
        if len(body) > 40:
            out.append({"file": rel, "line": start + 1, "text": f"// {rel}\n{body}"})
        i = max(end, i + 1)
    return out


def corpus():
    all_chunks = []
    for path in sorted(ROOT.rglob("*.rs")):
        if "tests" in path.parts:
            continue
        all_chunks.extend(chunks_of(path))
    return all_chunks


WORD = re.compile(r"[A-Za-zÀ-ÿ0-9]+")


def tokens(text):
    out = []
    for word in WORD.findall(text):
        for part in re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", word).split():
            out.append(part.lower())
    # Identifiers split on underscores too (WORD already did).
    return out


class BM25:
    def __init__(self, docs, k1=1.2, b=0.75):
        self.k1, self.b = k1, b
        self.docs = [Counter(tokens(d)) for d in docs]
        self.len = [sum(c.values()) for c in self.docs]
        self.avg = sum(self.len) / len(self.len)
        self.df = Counter()
        for doc in self.docs:
            self.df.update(doc.keys())
        self.n = len(docs)

    def scores(self, query):
        out = np.zeros(self.n)
        for term in tokens(query):
            df = self.df.get(term, 0)
            if not df:
                continue
            idf = math.log(1 + (self.n - df + 0.5) / (df + 0.5))
            for i, doc in enumerate(self.docs):
                tf = doc.get(term, 0)
                if tf:
                    norm = tf + self.k1 * (1 - self.b + self.b * self.len[i] / self.avg)
                    out[i] += idf * tf * (self.k1 + 1) / norm
        return out


def embed(texts):
    body = json.dumps({"model": MODEL, "input": texts, "truncate": True}).encode()
    req = urllib.request.Request(
        "http://127.0.0.1:11434/api/embed", body, {"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=600) as resp:
        return json.load(resp)["embeddings"]


def embed_all(texts, batch=64):
    out = []
    for i in range(0, len(texts), batch):
        out.extend(embed(texts[i : i + batch]))
    v = np.array(out, dtype=np.float32)
    return v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-9)


def rank_of(order, chunks, wanted, depth):
    """Position (from 1) of the first chunk in the right file within depth."""
    seen = 0
    for i in order[:depth]:
        seen += 1
        if wanted in chunks[i]["file"]:
            return seen
    return None


def rrf(*orders, k=60):
    score = defaultdict(float)
    for order in orders:
        for rank, i in enumerate(order[:200]):
            score[i] += 1 / (k + rank + 1)
    return sorted(score, key=score.get, reverse=True)


def report(name, ranks, ms=None):
    n = len(ranks)
    hit1 = sum(1 for r in ranks if r == 1) / n
    hit5 = sum(1 for r in ranks if r and r <= 5) / n
    hit10 = sum(1 for r in ranks if r and r <= 10) / n
    mrr = sum(1 / r for r in ranks if r) / n
    extra = f"  {ms:.1f} ms/query" if ms is not None else ""
    print(f"{name:<22} hit@1 {hit1:.0%}  hit@5 {hit5:.0%}  hit@10 {hit10:.0%}  MRR {mrr:.2f}{extra}")


chunks = corpus()
print(f"{len(chunks)} chunks, {sum(len(c['text']) for c in chunks) // 1024} KiB of code")
texts = [c["text"] for c in chunks]

t = time.time()
bm25 = BM25(texts)
print(f"BM25 index: {time.time() - t:.2f}s")

t = time.time()
docs = embed_all([DOC_PREFIX + x[:4000] for x in texts])
elapsed = time.time() - t
print(
    f"{MODEL}: {docs.shape[1]} dims, indexed in {elapsed:.1f}s "
    f"({len(chunks) / elapsed:.0f} chunks/s), {docs.nbytes // 1024} KiB as float32, "
    f"{docs.shape[0] * docs.shape[1] // 1024} KiB as int8"
)

keyword_ranks, vector_ranks, hybrid_ranks, int8_ranks = [], [], [], []
keyword_ms = vector_ms = 0.0
quantized = (docs * 127).astype(np.int8)
misses = []
for query, wanted in QUERIES:
    t = time.time()
    by_keyword = list(np.argsort(-bm25.scores(query)))
    keyword_ms += (time.time() - t) * 1000
    qv = embed_all([QUERY_PREFIX + query])[0]
    t = time.time()
    by_vector = list(np.argsort(-(docs @ qv)))
    vector_ms += (time.time() - t) * 1000
    by_int8 = list(np.argsort(-(quantized.astype(np.int32) @ (qv * 127).astype(np.int32))))
    fused = rrf(by_keyword, by_vector)
    keyword_ranks.append(rank_of(by_keyword, chunks, wanted, 50))
    vector_ranks.append(rank_of(by_vector, chunks, wanted, 50))
    int8_ranks.append(rank_of(by_int8, chunks, wanted, 50))
    hybrid_ranks.append(rank_of(fused, chunks, wanted, 50))
    print(
        f"  {query[:52]:<54} kw {str(keyword_ranks[-1]):>4}  vec {str(vector_ranks[-1]):>4}  "
        f"both {str(hybrid_ranks[-1]):>4}  -> {chunks[fused[0]]['file']}:{chunks[fused[0]]['line']}"
    )

print()
n = len(QUERIES)
report("keywords (BM25)", keyword_ranks, keyword_ms / n)
report("embeddings", vector_ranks, vector_ms / n)
report("embeddings int8", int8_ranks)
report("both (RRF)", hybrid_ranks)
