#!/usr/bin/env python3
"""Build Kor's model catalog (Master Build Prompt §4.4, ADR-0003).

Standard library only. Sources, in order:
  1. OpenRouter public /api/v1/models (no key): prices, context, tool calling, vision.
  2. Hugging Face public model API (no key): licence of open-weight models OpenRouter links.
  3. Artificial Analysis free data API, only when AA_API_KEY is set (CI secret, the only AA key Kor
     uses; apps never call AA): intelligence/coding/math index, median speed and time to first token.
     When AA fails or the key is missing, AA values from --previous (the last published catalog)
     are carried forward with their old `aa_fetched` time.

Every value records its source in the model's `source` map. A value no source gives stays null.

  python3 catalog/build_catalog.py --out catalog.json [--previous old/catalog.json]
  python3 catalog/build_catalog.py --self-test
"""

import argparse
import datetime
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

OPENROUTER_URL = "https://openrouter.ai/api/v1/models"
HF_URL = "https://huggingface.co/api/models/"
AA_URL = "https://artificialanalysis.ai/api/v2/language/models/free"
# §4.4: the Open LLM Leaderboard's rows through the Hugging Face datasets-server (public, no key).
# LMArena and LiveBench publish no machine-readable endpoint (checked); they are not included.
HF_LB_URL = "https://datasets-server.huggingface.co/rows?dataset=open-llm-leaderboard/contents&config=default&split=train"
HF_LB_SRC = "open-llm-leaderboard"
HF_LB_PAGE = 100
HF_LB_MAX_PAGES = 80
USER_AGENT = "kor-model-catalog (+https://github.com/LLCForge/Kor)"
MAX_BYTES = 32 * 1024 * 1024
HF_MAX_LOOKUPS = 400
AA_MAX_PAGES = 20
AA_SRC = "artificial-analysis"
AA_FIELDS = [
    "aa_intelligence_index", "aa_coding_index", "aa_math_index", "speed_tps", "ttft_ms",
    "input_price_per_m", "output_price_per_m",
]

FIELDS = [
    "model_id", "provider", "aa_intelligence_index", "aa_coding_index", "aa_math_index",
    "speed_tps", "ttft_ms", "input_price_per_m", "output_price_per_m", "context", "tool_calling",
    "vision", "license", "hf_leaderboard_avg", "updated_at", "aa_fetched", "source",
]


def get_json(url, headers=None):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, **(headers or {})})
    with urllib.request.urlopen(req, timeout=30) as resp:
        body = resp.read(MAX_BYTES + 1)
    if len(body) > MAX_BYTES:
        raise ValueError(f"{url}: response larger than {MAX_BYTES} bytes")
    return json.loads(body)


def empty(model_id, provider, now):
    m = {f: None for f in FIELDS}
    m.update(model_id=model_id, provider=provider, updated_at=now, source={})
    return m


def put(m, field, value, source):
    if value is None:
        return
    m[field] = value
    m["source"][field] = source


def per_million(price):
    """OpenRouter prices are USD per token as strings; negative means "varies"."""
    try:
        p = float(price)
    except (TypeError, ValueError):
        return None
    return None if p < 0 else round(p * 1_000_000, 6)


def norm(name):
    """`anthropic/claude-3.5-sonnet:free` and `claude-35-sonnet` both become `claude35sonnet`."""
    name = name.split("/")[-1].split(":")[0]
    return re.sub(r"[^a-z0-9]", "", name.lower())


def from_openrouter(doc, now):
    out = []
    for r in doc.get("data") or []:
        mid = r.get("id")
        if not isinstance(mid, str) or not mid:
            continue
        m = empty(mid, mid.split("/")[0] if "/" in mid else None, now)
        pricing = r.get("pricing") or {}
        put(m, "input_price_per_m", per_million(pricing.get("prompt")), "openrouter")
        put(m, "output_price_per_m", per_million(pricing.get("completion")), "openrouter")
        ctx = r.get("context_length")
        put(m, "context", ctx if isinstance(ctx, int) and ctx > 0 else None, "openrouter")
        params = r.get("supported_parameters")
        if isinstance(params, list):
            put(m, "tool_calling", "tools" in params, "openrouter")
        modalities = (r.get("architecture") or {}).get("input_modalities")
        if isinstance(modalities, list):
            put(m, "vision", "image" in modalities, "openrouter")
        hf = r.get("hugging_face_id")
        m["_hf"] = hf if isinstance(hf, str) and hf else None
        out.append(m)
    return out


def add_licenses(models):
    seen = {}
    for m in models:
        hf = m.get("_hf")
        if not hf or not re.fullmatch(r"[\w.-]+/[\w.-]+", hf):
            continue
        if hf not in seen:
            if len(seen) >= HF_MAX_LOOKUPS:
                continue
            try:
                info = get_json(HF_URL + urllib.parse.quote(hf, safe="/"))
            except (urllib.error.URLError, ValueError, OSError) as e:
                print(f"huggingface {hf}: {e}", file=sys.stderr)
                info = {}
            lic = (info.get("cardData") or {}).get("license")
            if not isinstance(lic, str):
                tags = [t for t in info.get("tags") or [] if isinstance(t, str)]
                lic = next((t[8:] for t in tags if t.startswith("license:")), None)
            seen[hf] = lic
        put(m, "license", seen[hf], "huggingface")


def fetch_hf_leaderboard():
    """Model repo id (lower case) -> the leaderboard's average score (0-100), best of its rows."""
    scores = {}
    for page in range(HF_LB_MAX_PAGES):
        doc = get_json(f"{HF_LB_URL}&offset={page * HF_LB_PAGE}&length={HF_LB_PAGE}")
        rows = doc.get("rows") or []
        for r in rows:
            row = r.get("row") or {}
            name = row.get("fullname")
            avg = next((num(v) for k, v in row.items() if isinstance(k, str) and k.startswith("Average")), None)
            if isinstance(name, str) and avg is not None:
                key = name.lower()
                scores[key] = max(avg, scores.get(key, avg))
        if len(rows) < HF_LB_PAGE:
            break
    return scores


def merge_hf_leaderboard(models, scores, prev):
    """Scores by the model's Hugging Face id; when the fetch failed, last catalog's values."""
    old = {m.get("model_id"): m for m in (prev or {}).get("models") or []}
    n = 0
    for m in models:
        hf = (m.get("_hf") or "").lower()
        if scores is not None and hf in scores:
            put(m, "hf_leaderboard_avg", round(scores[hf], 2), HF_LB_SRC)
            n += 1
        elif scores is None:
            o = old.get(m["model_id"]) or {}
            if num(o.get("hf_leaderboard_avg")) is not None:
                put(m, "hf_leaderboard_avg", o["hf_leaderboard_avg"], HF_LB_SRC)
                n += 1
    return n


def num(v):
    return v if isinstance(v, (int, float)) and not isinstance(v, bool) else None


def merge_aa(models, rows, now, label=AA_SRC):
    by_id = {m["model_id"]: m for m in models}
    by_norm = {}
    for m in models:
        by_norm.setdefault(norm(m["model_id"]), m)
    for a in rows:
        slug = a.get("slug")
        if not isinstance(slug, str) or not slug:
            continue
        m = by_id.get(a.get("openrouter_api_id") or "") or by_norm.get(norm(slug))
        if m is None:
            creator = (a.get("model_creator") or {}).get("name")
            m = empty(slug, creator if isinstance(creator, str) else None, now)
            models.append(m)
            by_norm[norm(slug)] = m
        ev = a.get("evaluations") or {}
        perf = a.get("performance") or {}
        put(m, "aa_intelligence_index", num(ev.get("artificial_analysis_intelligence_index")), label)
        put(m, "aa_coding_index", num(ev.get("artificial_analysis_coding_index")), label)
        put(m, "aa_math_index", num(ev.get("artificial_analysis_math_index")), label)
        put(m, "speed_tps", num(perf.get("median_output_tokens_per_second")), label)
        ttft = num(perf.get("median_time_to_first_token_seconds"))
        put(m, "ttft_ms", round(ttft * 1000, 1) if ttft is not None else None, label)
        pricing = a.get("pricing") or {}
        if m["input_price_per_m"] is None:
            put(m, "input_price_per_m", num(pricing.get("price_1m_input_tokens")), label)
        if m["output_price_per_m"] is None:
            put(m, "output_price_per_m", num(pricing.get("price_1m_output_tokens")), label)
        m["aa_fetched"] = now


def carry_aa(models, prev, now):
    """Copy AA values from the previous catalog, keeping their old `aa_fetched`; returns the count."""
    by_id = {m["model_id"]: m for m in models}
    n = 0
    for p in (prev or {}).get("models") or []:
        if not isinstance(p, dict):
            continue
        mid, fetched, src = p.get("model_id"), p.get("aa_fetched"), p.get("source")
        if not (isinstance(mid, str) and mid and isinstance(fetched, str) and len(fetched) <= 40
                and isinstance(src, dict)):
            continue
        m = by_id.get(mid)
        if m is None:
            prov = p.get("provider")
            m = by_id[mid] = empty(mid, prov if isinstance(prov, str) else None, now)
            models.append(m)
        for f in AA_FIELDS:
            if src.get(f) == AA_SRC and m[f] is None:
                put(m, f, num(p.get(f)), AA_SRC)
        m["aa_fetched"] = fetched
        n += 1
    return n


def fetch_aa(key):
    rows = []
    for page in range(1, AA_MAX_PAGES + 1):
        doc = get_json(f"{AA_URL}?page={page}", {"x-api-key": key})
        rows.extend(r for r in doc.get("data") or [] if isinstance(r, dict))
        if not (doc.get("pagination") or {}).get("has_more"):
            break
    return rows


def build(prev=None):
    now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()
    models = from_openrouter(get_json(OPENROUTER_URL), now)
    add_licenses(models)
    try:
        lb = fetch_hf_leaderboard()
    except (urllib.error.URLError, ValueError, OSError) as e:
        print(f"open-llm-leaderboard: {type(e).__name__}: {e}", file=sys.stderr)
        lb = None
    n = merge_hf_leaderboard(models, lb, prev)
    print(f"open-llm-leaderboard: {n} models", file=sys.stderr)
    for m in models:
        m.pop("_hf", None)
    key = os.environ.get("AA_API_KEY", "").strip()
    fresh = False
    if key:
        try:
            merge_aa(models, fetch_aa(key), now)
            fresh = True
        except (urllib.error.URLError, ValueError, OSError) as e:
            # HTTPError/URLError text never holds request headers, so the key stays out of the log.
            print(f"artificial-analysis: {type(e).__name__}: {e}", file=sys.stderr)
    else:
        print("artificial-analysis: AA_API_KEY not set", file=sys.stderr)
    if not fresh:
        n = carry_aa(models, prev, now)
        print(f"artificial-analysis: carried {n} models forward from the previous catalog", file=sys.stderr)
    models.sort(key=lambda m: m["model_id"])
    return {
        "schema": 1,
        "generated_at": now,
        "attribution": "Data: Artificial Analysis https://artificialanalysis.ai/ ; prices: https://openrouter.ai/ ; Open LLM Leaderboard https://huggingface.co/open-llm-leaderboard",
        "models": models,
    }


def load_prev(path):
    """The last published catalog, or None when missing or unreadable (first run, new fork)."""
    if not path:
        return None
    try:
        with open(path, encoding="utf-8") as f:
            if os.fstat(f.fileno()).st_size > MAX_BYTES:
                raise ValueError("larger than MAX_BYTES")
            doc = json.load(f)
        return doc if isinstance(doc, dict) else None
    except (OSError, ValueError) as e:
        print(f"previous catalog {path}: {e}", file=sys.stderr)
        return None


def dump(cat):
    text = json.dumps(cat, ensure_ascii=False, indent=1) + "\n"
    key = os.environ.get("AA_API_KEY", "").strip()
    if key and key in text:
        raise SystemExit("refusing to write: AA_API_KEY found in the output")
    return text


def self_test():
    now = "2026-01-01T00:00:00+00:00"
    doc = {"data": [
        {"id": "anthropic/claude-3.5-sonnet", "context_length": 200000,
         "pricing": {"prompt": "0.000003", "completion": "0.000015"},
         "supported_parameters": ["tools", "temperature"],
         "architecture": {"input_modalities": ["text", "image"]}},
        {"id": "openrouter/auto", "pricing": {"prompt": "-1", "completion": "-1"}},
    ]}
    ms = from_openrouter(doc, now)
    assert ms[0]["input_price_per_m"] == 3.0 and ms[0]["output_price_per_m"] == 15.0
    assert ms[0]["tool_calling"] is True and ms[0]["vision"] is True
    assert ms[0]["source"]["context"] == "openrouter"
    assert ms[1]["input_price_per_m"] is None and ms[1]["tool_calling"] is None
    assert "input_price_per_m" not in ms[1]["source"]
    merge_aa(ms, [
        {"slug": "claude-35-sonnet", "evaluations": {"artificial_analysis_intelligence_index": 44.1,
         "artificial_analysis_coding_index": None},
         "performance": {"median_output_tokens_per_second": 80.5,
                         "median_time_to_first_token_seconds": 0.9}},
        {"slug": "new-model", "model_creator": {"name": "Acme"},
         "pricing": {"price_1m_input_tokens": 1, "price_1m_output_tokens": 2}},
    ], now)
    assert ms[0]["aa_intelligence_index"] == 44.1 and ms[0]["ttft_ms"] == 900.0
    assert ms[0]["aa_coding_index"] is None and "aa_coding_index" not in ms[0]["source"]
    assert ms[0]["source"]["speed_tps"] == "artificial-analysis"
    assert ms[2]["model_id"] == "new-model" and ms[2]["input_price_per_m"] == 1
    assert ms[0]["aa_fetched"] == now and ms[1]["aa_fetched"] is None
    assert all(set(m) == set(FIELDS) | {"_hf"} for m in ms[:2])
    self_test_build()
    print("self-test ok")


def self_test_build():
    """build() on fixtures: AA succeeds (two pages, keyed), then AA is rate-limited and carries forward."""
    global get_json
    key = "aa-self-test-key-0123456789"
    aa_pages = {
        1: {"data": [{"slug": "claude-35-sonnet",
                      "evaluations": {"artificial_analysis_intelligence_index": 44.1,
                                      "artificial_analysis_coding_index": 38.0,
                                      "artificial_analysis_math_index": 51.5},
                      "performance": {"median_output_tokens_per_second": 80.5,
                                      "median_time_to_first_token_seconds": 0.9}}],
            "pagination": {"has_more": True}},
        2: {"data": [{"slug": "aa-only", "model_creator": {"name": "Acme"},
                      "evaluations": {"artificial_analysis_intelligence_index": 20}}],
            "pagination": {"has_more": False}},
    }
    seen = []
    aa_down = [False]

    def fake(url, headers=None):
        seen.append(url)
        if url == OPENROUTER_URL:
            return {"data": [{"id": "anthropic/claude-3.5-sonnet", "pricing": {"prompt": "0.000003"}},
                             {"id": "meta-llama/llama-3-8b", "hugging_face_id": "meta-llama/Meta-Llama-3-8B"}]}
        if url.startswith(HF_URL):
            return {"cardData": {"license": "llama3"}}
        if url.startswith(HF_LB_URL):
            if aa_down[0]:
                raise urllib.error.URLError("datasets-server down")
            return {"rows": [{"row": {"fullname": "meta-llama/Meta-Llama-3-8B", "Average \u2b06\ufe0f": 13.4123}},
                             {"row": {"fullname": "other/model", "Average \u2b06\ufe0f": 50}}]}
        if url.startswith(AA_URL):
            assert (headers or {}).get("x-api-key") == key and key not in url
            if aa_down[0]:
                raise urllib.error.HTTPError(url, 429, "Too Many Requests", None, None)
            return aa_pages[int(url.rsplit("=", 1)[1])]
        raise AssertionError(url)

    real, old_env = get_json, os.environ.get("AA_API_KEY")
    get_json, os.environ["AA_API_KEY"] = fake, key
    try:
        day1 = json.loads(dump(build()))
        assert sum(u.startswith(AA_URL) for u in seen) == 2
        a = {m["model_id"]: m for m in day1["models"]}
        c = a["anthropic/claude-3.5-sonnet"]
        assert (c["aa_intelligence_index"], c["aa_coding_index"], c["aa_math_index"]) == (44.1, 38.0, 51.5)
        assert c["source"]["aa_math_index"] == AA_SRC and c["input_price_per_m"] == 3.0
        assert c["aa_fetched"] == day1["generated_at"] and a["aa-only"]["aa_fetched"]
        assert "artificialanalysis.ai" in day1["attribution"]
        llama = a["meta-llama/llama-3-8b"]
        assert llama["hf_leaderboard_avg"] == 13.41 and llama["source"]["hf_leaderboard_avg"] == HF_LB_SRC
        assert llama["license"] == "llama3" and "_hf" not in llama
        old = "2026-01-01T00:00:00+00:00"
        for m in day1["models"]:
            m["aa_fetched"] = old
        aa_down[0] = True
        day2 = json.loads(dump(build(day1)))
        b = {m["model_id"]: m for m in day2["models"]}
        c2 = b["anthropic/claude-3.5-sonnet"]
        assert c2["aa_intelligence_index"] == 44.1 and c2["aa_math_index"] == 51.5 and c2["ttft_ms"] == 900.0
        assert c2["aa_fetched"] == old and c2["source"]["speed_tps"] == AA_SRC
        assert c2["source"]["input_price_per_m"] == "openrouter"
        assert b["aa-only"]["aa_intelligence_index"] == 20 and b["aa-only"]["provider"] == "Acme"
        assert b["meta-llama/llama-3-8b"]["hf_leaderboard_avg"] == 13.41, "carried forward"
        # No previous catalog either: AA fields stay empty, the keyless data still ships.
        assert build(None)["models"][0]["aa_intelligence_index"] is None
        try:
            dump({"leak": key})
            raise AssertionError("key written to the output")
        except SystemExit:
            pass
    finally:
        get_json = real
        if old_env is None:
            os.environ.pop("AA_API_KEY", None)
        else:
            os.environ["AA_API_KEY"] = old_env


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="catalog.json")
    ap.add_argument("--previous", help="last published catalog.json, for AA carry-forward")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        self_test()
        return
    cat = build(load_prev(a.previous))
    text = dump(cat)
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"{len(cat['models'])} models -> {a.out}")


if __name__ == "__main__":
    main()
