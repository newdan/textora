#!/usr/bin/env bash
# Generate the deterministic sample corpus described in plans.md §9.
# Idempotent: rerunning produces byte-identical output.
# Requires: bash, python3 (macOS system python is fine).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/assets/samples"
mkdir -p "$OUT"

# Python helper: stream content to stdout, no RNG, deterministic.
gen() {
    python3 - "$@" <<'PY'
import os, sys

mode = sys.argv[1]
out_path = sys.argv[2]
size = int(sys.argv[3]) if len(sys.argv) > 3 else 0

LOREM = (b"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod "
         b"tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, "
         b"quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.\n")

CJK = ("春眠不觉晓，处处闻啼鸟。夜来风雨声，花落知多少。"
       "床前明月光，疑是地上霜。举头望明月，低头思故乡。"
       "白日依山尽，黄河入海流。欲穷千里目，更上一层楼。\n").encode("utf-8")

EMOJI_ZWJ = ("👨‍👩‍👧‍👦 family. "
             "👋\U0001f3fb skin tone. "
             "🏳️‍🌈 flag. "
             "👨‍💻 worker.\n").encode("utf-8")

COMBINING_NFC = "café naïve résumé 한글\n".encode("utf-8")
COMBINING_NFD = ("café naïve résumé "
                 "간글\n").encode("utf-8")

ZERO_WIDTH = ("a​b‌c‍d "        # ZWSP/ZWNJ/ZWJ
              "☃︎ snowman text "      # variation selector
              "☃️ snowman emoji\n"    # variation selector
              ).encode("utf-8")

ARABIC = "السلام عليكم ورحمة الله\n".encode("utf-8")
HEBREW = "שלום עולם\n".encode("utf-8")
RTL = ARABIC + HEBREW

BOM = b"\xef\xbb\xbf"

def repeat_to(buf, target):
    """Return bytes of length exactly `target`, repeating buf and slicing."""
    if target == 0:
        return b""
    n = (target + len(buf) - 1) // len(buf)
    return (buf * n)[:target]

def write(data):
    with open(out_path, "wb") as f:
        f.write(data)

if mode == "empty":
    write(b"")
elif mode == "one-byte":
    write(b"a")
elif mode == "no-eol":
    write(b"this file has no trailing newline at the end of it")
elif mode == "ascii":
    write(repeat_to(LOREM, size))
elif mode == "ascii-bom":
    body = repeat_to(LOREM, size - len(BOM))
    write(BOM + body)
elif mode == "crlf":
    body = LOREM.replace(b"\n", b"\r\n")
    write(repeat_to(body, size))
elif mode == "cr-only":
    body = LOREM.replace(b"\n", b"\r")
    write(repeat_to(body, size))
elif mode == "mixed-eol":
    parts = [
        b"line 1 LF\n",
        b"line 2 CRLF\r\n",
        b"line 3 CR\r",
        b"line 4 LF\n",
        b"line 5 CRLF\r\n",
    ]
    block = b"".join(parts)
    write(repeat_to(block, size))
elif mode == "cjk":
    write(repeat_to(CJK, size))
elif mode == "emoji-zwj":
    write(repeat_to(EMOJI_ZWJ, size))
elif mode == "combining":
    block = COMBINING_NFC + COMBINING_NFD
    write(repeat_to(block, size))
elif mode == "zero-width":
    write(repeat_to(ZERO_WIDTH, size))
elif mode == "rtl":
    write(repeat_to(RTL, size))
elif mode == "illegal-utf8":
    # mix of valid ASCII and intentionally invalid byte sequences
    block = (b"valid ascii line\n"
             b"\xc3\x28 invalid 2-byte\n"      # bad continuation
             b"\xe2\x82\x28 invalid 3-byte\n"
             b"\xf0\x90\x28\xbc invalid 4-byte\n"
             b"\xff\xfe stray\n"
             b"\xed\xa0\x80 surrogate\n")
    write(repeat_to(block, size))
elif mode == "long-line":
    body = b"x" * (size - 1) + b"\n"
    write(body)
elif mode == "long-line-no-eol":
    write(b"x" * size)
elif mode == "binary-nulls":
    block = bytearray()
    for i in range(size):
        # interleave \0 with printable bytes deterministically
        block.append(0 if (i % 8 == 0) else 0x41 + (i % 26))
    write(bytes(block))
else:
    raise SystemExit(f"unknown mode: {mode}")
PY
}

# Tiny
gen empty       "$OUT/tiny_empty.txt"
gen one-byte    "$OUT/tiny_one_byte.txt"
gen no-eol      "$OUT/tiny_no_eol.txt"

# Small (4 KB)
gen ascii       "$OUT/small_ascii.txt"        4096
gen crlf        "$OUT/small_crlf.txt"         4096
gen cr-only     "$OUT/small_cr_only.txt"      4096
gen mixed-eol   "$OUT/small_mixed_eol.txt"    4096
gen ascii-bom   "$OUT/small_bom.txt"          4096

# Small (16 KB CJK, 8 KB others)
gen cjk         "$OUT/small_cjk.txt"          16384
gen emoji-zwj   "$OUT/small_emoji_zwj.txt"    8192
gen combining   "$OUT/small_combining.txt"    8192
gen zero-width  "$OUT/small_zero_width.txt"   4096
gen rtl         "$OUT/small_rtl.txt"          4096
gen illegal-utf8 "$OUT/small_illegal_utf8.bin" 4096

# Medium / large
gen ascii       "$OUT/medium_ascii_5mb.txt"   5242880
gen ascii       "$OUT/large_ascii_50mb.txt"   52428800
gen cjk         "$OUT/large_cjk_50mb.txt"     52428800
gen ascii       "$OUT/huge_ascii_200mb.txt"   209715200

# Long lines
gen long-line          "$OUT/long_line_1mb.txt"        1048576
gen long-line-no-eol   "$OUT/long_line_no_eol.txt"     1048576

# Edge
gen binary-nulls "$OUT/binary_with_nulls.bin" 8192
gen ascii        "$OUT/path_with_spaces 中文 🌏.txt"  4096

# Symlink (idempotent: remove + recreate)
ln -sf small_ascii.txt "$OUT/symlink_to_small.txt"

# Readonly
gen ascii        "$OUT/readonly.txt"          4096
chmod 0444 "$OUT/readonly.txt"

# Compute SHA256 manifest (skip the symlink's contents to keep this stable;
# also skip readonly to avoid permission tweaks before checksumming)
cd "$OUT"
shasum -a 256 \
    tiny_empty.txt \
    tiny_one_byte.txt \
    tiny_no_eol.txt \
    small_ascii.txt \
    small_crlf.txt \
    small_cr_only.txt \
    small_mixed_eol.txt \
    small_bom.txt \
    small_cjk.txt \
    small_emoji_zwj.txt \
    small_combining.txt \
    small_zero_width.txt \
    small_rtl.txt \
    small_illegal_utf8.bin \
    medium_ascii_5mb.txt \
    large_ascii_50mb.txt \
    large_cjk_50mb.txt \
    huge_ascii_200mb.txt \
    long_line_1mb.txt \
    long_line_no_eol.txt \
    binary_with_nulls.bin \
    "path_with_spaces 中文 🌏.txt" \
    readonly.txt \
    > SHA256SUMS

echo "samples generated at $OUT"
wc -c SHA256SUMS
