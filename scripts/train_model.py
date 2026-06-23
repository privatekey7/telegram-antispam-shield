#!/usr/bin/env python3
"""Train the anti-spam classifier and export a portable model for the Rust bot.

The model is a linear classifier (logistic regression) over hashed text
features. Feature extraction here MUST stay byte-for-byte compatible with the
Rust implementation in `src/features.rs` — same normalization, same n-grams,
same FNV-1a hash, same bucket count. That parity is what lets us train in
Python and serve in Rust.

Output: `model/spam_model.bin` (see FORMAT below) + `model/metrics.json`.

Run:
    python3 scripts/train_model.py
"""
from __future__ import annotations

import json
import re
import struct
import time
from pathlib import Path

import numpy as np
from scipy.sparse import csr_matrix
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import (
    classification_report,
    confusion_matrix,
    precision_recall_curve,
    roc_auc_score,
)
from sklearn.model_selection import train_test_split

# --- Feature-extraction spec (keep in sync with src/features.rs) -------------
NUM_BUCKETS = 1 << 20  # 2^20 buckets -> 4 MB of f32 weights
CHAR_MIN, CHAR_MAX = 3, 5  # character n-gram range (subword signal)
WORD_NGRAM_MAX = 2  # word unigrams + bigrams
MAX_CHARS = 800  # truncate very long messages (spam is short; bounds memory)
WORD_RE = re.compile(r"[^\W_]+", re.UNICODE)

# --- Model binary format ----------------------------------------------------
# magic[4]="SASM" | version:u32 | num_buckets:u32 |
# char_min:u8 char_max:u8 word_ngram_max:u8 _pad:u8 |
# bias:f32 | threshold:f32 | weights: num_buckets * f32   (all little-endian)
MAGIC = b"SASM"
VERSION = 1

ROOT = Path(__file__).resolve().parent.parent
MODEL_DIR = ROOT / "model"
CACHE = ROOT / "scripts" / ".cache"


def fnv1a(s: str) -> int:
    """64-bit FNV-1a hash. Must match the Rust implementation exactly."""
    h = 0xCBF29CE484222325
    for byte in s.encode("utf-8"):
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def feature_buckets(text: str) -> set[int]:
    """Extract hashed feature buckets for one message (binary presence)."""
    text = text.lower()[:MAX_CHARS]
    words = WORD_RE.findall(text)
    feats: set[str] = set()
    for i, w in enumerate(words):
        feats.add("w1:" + w)
        if WORD_NGRAM_MAX >= 2 and i + 1 < len(words):
            feats.add("w2:" + w + " " + words[i + 1])
    for w in words:
        cw = "<" + w + ">"
        for n in range(CHAR_MIN, CHAR_MAX + 1):
            if len(cw) >= n:
                for i in range(len(cw) - n + 1):
                    feats.add("c:" + cw[i : i + n])
    return {fnv1a(f) % NUM_BUCKETS for f in feats}


def load_corpus() -> "tuple[list[str], np.ndarray]":
    """Load + merge public RU/EN spam datasets, dedup, cache to parquet."""
    import pandas as pd

    CACHE.mkdir(parents=True, exist_ok=True)
    cache_file = CACHE / "corpus.parquet"
    if cache_file.exists():
        df = pd.read_parquet(cache_file)
        print(f"[data] loaded cached corpus: {len(df)} rows")
        return df["text"].tolist(), df["label"].to_numpy()

    import os

    os.environ["HF_HUB_DISABLE_PROGRESS_BARS"] = "1"
    from datasets import load_dataset

    frames = []
    # 1) Telegram spam, Russian + English (primary)
    d = load_dataset("alt-gnome/telegram-spam", split="train").to_pandas()
    frames.append(pd.DataFrame({"text": d["text"], "label": d["label"].astype(int)}))
    # 2) SMS / Enron / Telegram mix (reinforces English)
    d = load_dataset("mshenoda/spam-messages", split="train").to_pandas()
    lbl = (d["label"].astype(str).str.lower() == "spam").astype(int)
    frames.append(pd.DataFrame({"text": d["text"], "label": lbl}))

    df = pd.concat(frames, ignore_index=True)
    df["text"] = df["text"].astype(str).str.strip()
    df = df[df["text"].str.len() > 0]
    df = df.drop_duplicates(subset=["text"]).reset_index(drop=True)
    df.to_parquet(cache_file)
    print(f"[data] built corpus: {len(df)} rows (cached -> {cache_file})")
    return df["text"].tolist(), df["label"].to_numpy()


def build_matrix(texts: "list[str]") -> csr_matrix:
    t0 = time.time()
    indptr = [0]
    indices: list[int] = []
    for t in texts:
        indices.extend(feature_buckets(t))
        indptr.append(len(indices))
    data = np.ones(len(indices), dtype=np.float32)
    X = csr_matrix(
        (data, np.asarray(indices, dtype=np.int32), np.asarray(indptr, dtype=np.int64)),
        shape=(len(texts), NUM_BUCKETS),
        dtype=np.float32,
    )
    print(f"[feat] matrix {X.shape} nnz={X.nnz} in {time.time() - t0:.1f}s")
    return X


def pick_threshold(y_true: np.ndarray, proba: np.ndarray, max_fpr: float = 0.01) -> float:
    """Choose the lowest threshold whose false-positive rate <= max_fpr.

    We strongly prefer NOT deleting legitimate messages, so we cap the
    false-positive rate and maximize spam recall under that constraint.
    """
    prec, rec, thr = precision_recall_curve(y_true, proba)
    neg = y_true == 0
    n_neg = max(int(neg.sum()), 1)
    best = 0.5
    best_recall = -1.0
    for t in np.unique(np.round(thr, 4)):
        pred = proba >= t
        fp = int((pred & neg).sum())
        fpr = fp / n_neg
        if fpr <= max_fpr:
            tp = int((pred & (y_true == 1)).sum())
            recall = tp / max(int((y_true == 1).sum()), 1)
            if recall > best_recall:
                best_recall, best = recall, float(t)
    return best


def export_model(weights: np.ndarray, bias: float, threshold: float) -> Path:
    MODEL_DIR.mkdir(parents=True, exist_ok=True)
    out = MODEL_DIR / "spam_model.bin"
    w = weights.astype("<f4")
    with open(out, "wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<I", VERSION))
        f.write(struct.pack("<I", NUM_BUCKETS))
        f.write(struct.pack("<BBBB", CHAR_MIN, CHAR_MAX, WORD_NGRAM_MAX, 0))
        f.write(struct.pack("<f", float(bias)))
        f.write(struct.pack("<f", float(threshold)))
        f.write(w.tobytes())
    print(f"[export] wrote {out} ({out.stat().st_size / 1e6:.2f} MB)")
    return out


def main() -> None:
    texts, y = load_corpus()
    print(f"[data] ham={int((y == 0).sum())} spam={int((y == 1).sum())}")
    X = build_matrix(texts)

    Xtr, Xte, ytr, yte = train_test_split(
        X, y, test_size=0.2, random_state=42, stratify=y
    )
    clf = LogisticRegression(
        C=4.0, max_iter=1000, class_weight="balanced", solver="liblinear"
    )
    t0 = time.time()
    clf.fit(Xtr, ytr)
    print(f"[train] fit in {time.time() - t0:.1f}s")

    proba = clf.predict_proba(Xte)[:, 1]
    threshold = pick_threshold(yte, proba, max_fpr=0.01)
    pred = (proba >= threshold).astype(int)

    print(f"[eval] threshold={threshold:.4f}")
    print(classification_report(yte, pred, target_names=["ham", "spam"], digits=4))
    cm = confusion_matrix(yte, pred)
    auc = roc_auc_score(yte, proba)
    print("[eval] confusion (rows=true [ham,spam]):\n", cm)
    print(f"[eval] ROC-AUC={auc:.4f}")

    weights = clf.coef_.ravel().astype(np.float32)
    bias = float(clf.intercept_[0])
    export_model(weights, bias, threshold)

    metrics = {
        "rows": int(X.shape[0]),
        "ham": int((y == 0).sum()),
        "spam": int((y == 1).sum()),
        "num_buckets": NUM_BUCKETS,
        "threshold": round(threshold, 4),
        "roc_auc": round(float(auc), 4),
        "confusion_matrix": cm.tolist(),
        "report": classification_report(
            yte, pred, target_names=["ham", "spam"], output_dict=True
        ),
    }
    (MODEL_DIR / "metrics.json").write_text(json.dumps(metrics, indent=2, ensure_ascii=False))
    print(f"[export] wrote {MODEL_DIR / 'metrics.json'}")


if __name__ == "__main__":
    main()
