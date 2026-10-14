---
type: Fact
title: Scalar evidence
description: Preserve inert typed metadata.
generated: { by: human:alice, at: 2026-08-25T12:00:00Z }
vendor_extension:
  true_values: [true, True, TRUE]
  false_values: [false, False, FALSE]
  legacy_words: [yes, no, on, off, y, n]
  opaque_id: 0123
  decimal_separator: 1_000
  binary: 0b11
  octal: 0o17
  hex: 0xFF
  unsigned_max: 18446744073709551615
  signed_min: -9223372036854775808
  exponent: 1.2e3
  negative_zero: -0.0
  timestamp: 2026-08-20T10:00:00Z
  forced_string: !!str true
  string_nan: !!str .nan
  float_overflow: 1e999
  unicode: नमस्ते
  quoted: "foo *baz << !include ${HOME}"
  literal: |
    Keep literal text.
    !include /private/synthetic.yaml
  folded: >-
    Preserve
    words.
---
Body.
