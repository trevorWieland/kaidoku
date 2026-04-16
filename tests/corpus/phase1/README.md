# Phase 1 Fixture Corpus

This directory contains the committed fixture PDFs used by the Phase 1 parser gate.

## Source

Fixtures are sourced from the official DocLayNet benchmark extra archive:

- Dataset: DocLayNet v1.0.0
- License: CDLA-Permissive-1.0
- URL: https://codait-cos-dax.s3.us.cloud-object-storage.appdomain.cloud/dax-doclaynet/1.0.0/DocLayNet_extra.zip

See `provenance.json` for exact source ZIP paths, retrieval date, and SHA-256 checksums.
Additional multilingual fixtures are sourced from the Mozilla `pdf.js` test corpus (Apache-2.0)
with direct source URLs recorded in `provenance.json`.

## Naming convention

- `doclaynet_simple_text.pdf` — mostly textual page with no table/picture/formula annotations.
- `doclaynet_multi_column.pdf` — page with text regions spread across left and right columns.
- `doclaynet_mixed_content.pdf` — page with text + non-text content (pictures/captions).

Each fixture has a matching golden output JSON in `tests/golden/phase1/<fixture-stem>.json`.

Additional fixtures:

- `pdfjs_copy_paste_ligatures.pdf` — ligature-heavy text extraction edge-case fixture.
- `pdfjs_arabic_cid_true_type.pdf` — RTL/CID font fixture.
- `pdfjs_identity_to_unicode_map_char_code_of.pdf` — ToUnicode/CMap mapping fixture.
