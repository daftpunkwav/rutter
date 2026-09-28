#!/usr/bin/env bash
#
# Encoding gate: every tracked text file must be valid UTF-8, carry no
# byte-order mark, and use LF line endings — the repository normalizes
# to LF via .gitattributes (*.sh is forced LF on every platform).
# Content language is free (Chinese and English are both accepted);
# this gate checks byte hygiene only, never language. Binary files
# are skipped with the same heuristic git grep uses.
#
# One perl process scans every tracked file. Spawning four probes per
# file (a binary grep, a UTF-8 perl, head, a CRLF grep) made the gate
# wall-clock dominated by process startup on Windows; the checks and
# their order are unchanged, only the per-file spawning is gone.

set -u

root=$(git rev-parse --show-toplevel)
scan=$(mktemp)
trap 'rm -f "$scan"' EXIT

# The scan writes one failure line per offending file. Any nonzero exit
# status means the scan itself broke (no perl, disk error): the gate
# fails instead of silently passing an unscanned tree.
git -C "$root" ls-files -z | perl -e '
    use strict;
    use warnings;
    use Encode ();
    my $root = $ARGV[0];
    binmode STDIN;
    my $input = do { local $/; <STDIN> };
    for my $file (split /\0/, $input, -1) {
        next if $file eq "";
        my $path = "$root/$file";
        open my $fh, "<:raw", $path or next;
        my $data = do { local $/; <$fh> };
        close $fh;
        # Binary skip (the -I heuristic: a NUL in the first 32 KiB).
        next if index(substr($data, 0, 32768), "\0") >= 0;
        # UTF-8 validity: strict decode fails on malformed byte
        # sequences. Decode a COPY: with FB_CROAK, Encode::decode
        # modifies its argument in place, and the BOM and CRLF checks
        # below must see the raw bytes.
        my $decoded = $data;
        eval { Encode::decode("UTF-8", $decoded, Encode::FB_CROAK()) };
        if ($@) { print "$file: not valid UTF-8\n"; next; }
        # Byte-order mark.
        if (substr($data, 0, 3) eq "\xEF\xBB\xBF") {
            print "$file: UTF-8 byte-order mark\n";
            next;
        }
        # CRLF line endings: any carriage-return byte.
        if (index($data, "\r") >= 0) { print "$file: CRLF line endings\n"; }
    }
' "$root" > "$scan"
scan_status=${PIPESTATUS[1]}

if [[ $scan_status -ne 0 ]]; then
  printf 'encoding gate: scan failed\n' >&2
  exit 1
fi

failures=()
while IFS= read -r line; do
  failures+=("$line")
done < "$scan"

if [[ ${#failures[@]} -gt 0 ]]; then
  printf 'encoding gate: byte-hygiene failures:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi

printf 'encoding gate: clean\n'
