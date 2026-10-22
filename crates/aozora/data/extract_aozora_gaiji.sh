#!/usr/bin/env bash

set -euo pipefail

UPSTREAM="https://raw.githubusercontent.com/kurema/AozoraGaijiChukiXml/master/data/Chuki.xml"
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/aozora-gaiji-chuki.tsv"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "→ downloading $UPSTREAM"
curl -sLf --max-time 60 -o "$TMP/Chuki.xml" "$UPSTREAM"

echo "→ awk-extracting raw pairs"
awk '
  /<entry / { in_entry=1; char=""; desc=""; next }
  /<\/entry>/ {
    if (char != "" && desc != "") print desc "\t" char;
    in_entry=0; char=""; desc=""; next
  }
  in_entry && /<character>/ {
    if (char == "") {
      line = $0;
      sub(/.*<character>/, "", line);
      sub(/<\/character>.*/, "", line);
      char = line;
    }
    next
  }
  in_entry && /<description>/ {
    line = $0;
    sub(/.*<description>/, "", line);
    sub(/<\/description>.*/, "", line);
    sub(/^「/, "", line);
    sub(/」$/, "", line);
    desc = line;
    next
  }
' "$TMP/Chuki.xml" > "$TMP/raw.tsv"

echo "→ filter / decode / dedupe"
perl -CSD -E '
  my %seen;
  while (<>) {
    chomp;
    my ($desc, $char) = split /\t/, $_, 2;
    next unless defined $desc && defined $char;
    next if $desc eq "" || $char eq "";
    next unless length($char) == 1;            # exactly one codepoint
    next if $desc =~ /[\x00-\x1F]/;
    # decode XML entities (apply &amp; LAST so it does not double-decode)
    for my $s ($desc, $char) {
      $s =~ s/&lt;/</g;
      $s =~ s/&gt;/>/g;
      $s =~ s/&quot;/"/g;
      $s =~ s/&apos;/'\''/g;
      $s =~ s/&amp;/&/g;
    }
    next if $seen{$desc}++;
    say "$desc\t" . sprintf("%X", ord $char);
  }
' "$TMP/raw.tsv" > "$TMP/clean.tsv"

echo "→ writing $OUT"
{
  echo "# Aozora Bunko 外字注記辞書 — description → Unicode codepoint mapping."
  echo "#"
  echo "# Source: kurema/AozoraGaijiChukiXml (CC0 1.0), an XML transcription of the"
  echo "# official 青空文庫・外字注記辞書【第八版】 (PDF). Upstream URL:"
  echo "#   $UPSTREAM"
  echo "# License: CC0 1.0 Universal (Public Domain Dedication)."
  echo "#"
  echo "# Filtering applied during extraction:"
  echo "#  * exactly 1 Unicode codepoint per entry (multi-codepoint forms dropped)"
  echo "#  * surrounding 「」 stripped from descriptions"
  echo "#  * XML entities (&lt;, &gt;, &amp;) decoded"
  echo "#  * first-wins on duplicate descriptions"
  echo "#"
  echo "# Format: description<TAB>codepoint_hex"
  echo "# Regenerate from upstream: see extract_aozora_gaiji.sh in this directory."
  cat "$TMP/clean.tsv"
} > "$OUT"

LINES=$(grep -cv '^#' "$OUT")
echo "✓ wrote $OUT ($LINES entries)"
