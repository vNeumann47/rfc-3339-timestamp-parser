# rfc-3339-timestamp-parser

A small, dependency-free Rust library that parses RFC 3339 timestamps into a UTC instant.

```rust
use rfc_3339_timestamp_parser::{parse, Timestamp};

let ts = parse("2024-01-02T03:04:05.123+01:00").unwrap();
assert_eq!(ts.seconds, 1704161045);
assert_eq!(ts.nanos, 123_000_000);
```

## Why this exists

RFC 3339 is the timestamp format used by most web APIs, but the standard library has no parser for it. This crate provides one that is strict about the grammar and normalises every result to UTC, so you get a single, unambiguous instant regardless of the original offset.

The main trade-off is that leap seconds (`:60`) are rejected. RFC 3339 allows them, but representing them correctly requires a leap-second table, which is not available in `std` and would make the parser stateful. Rejecting them keeps the parser pure and the `Timestamp` type simple.

## Edge you will hit

Fractional seconds longer than nine digits are accepted but truncated to nanosecond precision. If you need sub-nanosecond accuracy this library is not for you.

## API

- `parse(input: &str) -> Result<Timestamp, ParseError>`
- `Timestamp { seconds: i64, nanos: u32 }`
- `Timestamp::new(seconds: i64, nanos: u32) -> Timestamp`
- `ParseError::{Empty, Format(String), OutOfRange(String)}`
