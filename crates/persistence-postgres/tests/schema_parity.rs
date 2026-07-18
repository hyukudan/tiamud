//! Cross-dialect schema drift detector for the two hand-maintained
//! persistence adapters.
//!
//! `tiamud-persistence-sqlite` and `tiamud-persistence-postgres` each carry the
//! full DDL history up to `SCHEMA_VERSION = 15` in a different SQL dialect.
//! Parity between them is otherwise guaranteed only by manual review, so a
//! table or column added to one adapter but forgotten in the other would drift
//! silently. This test compares the *set of `(table, column)` pairs* the two
//! adapters define at v15 and fails loudly on any asymmetry.
//!
//! Sides are obtained differently, on purpose:
//!
//! * SQLite: the **real** `migrate()` runs against an in-memory database and
//!   the resulting schema is read back through `sqlite_master` /
//!   `pragma_table_info`. This is authoritative execution, not parsing.
//! * PostgreSQL: this environment has no server (`TIAMUD_TEST_POSTGRES_URL` is
//!   unset), so the fresh-install `SCHEMA_SQL` string constant is parsed
//!   **statically**. This is STATIC DDL ANALYSIS, not real execution.
//!
//! Only table and column *names* are compared. Type differences across
//! dialects (`TEXT`/`TEXT`, `BLOB`/`BYTEA`, `INTEGER`/`BIGINT`, `AUTOINCREMENT`
//! vs `BIGSERIAL`, ...) are intentionally ignored.

use std::collections::BTreeSet;

type ColumnSet = BTreeSet<(String, String)>;

/// Tables that legitimately exist in exactly one dialect and therefore must be
/// excluded from the parity comparison. Each entry is tightly scoped and
/// justified; prefer failing over widening this list.
///
/// * `tiamud_schema` is PostgreSQL-only. SQLite tracks the schema version with
///   the built-in `PRAGMA user_version`, which is not a table, so there is no
///   SQLite counterpart to mirror. This is a genuine dialect artifact, not
///   drift.
const POSTGRES_ONLY_TABLES: &[&str] = &["tiamud_schema"];

#[test]
fn sqlite_and_postgres_define_the_same_tables_and_columns() {
    let sqlite = sqlite_columns_via_real_migration();
    let postgres = postgres_columns_via_static_ddl_parse();

    // Whole-table dialect artifacts are dropped from both observed sets so the
    // comparison sees only tables that are supposed to exist in both adapters.
    let postgres: ColumnSet = postgres
        .into_iter()
        .filter(|(table, _)| !POSTGRES_ONLY_TABLES.contains(&table.as_str()))
        .collect();
    let sqlite: ColumnSet = sqlite
        .into_iter()
        .filter(|(table, _)| !POSTGRES_ONLY_TABLES.contains(&table.as_str()))
        .collect();

    // Sanity: the parser must actually have found the schema. A silently empty
    // set would make the equality check pass vacuously.
    assert!(
        postgres.len() > 100,
        "static PostgreSQL DDL parse produced only {} columns; the parser is broken",
        postgres.len()
    );
    assert!(
        sqlite.len() > 100,
        "SQLite introspection produced only {} columns; migration or introspection is broken",
        sqlite.len()
    );

    let only_in_sqlite: Vec<_> = sqlite.difference(&postgres).collect();
    let only_in_postgres: Vec<_> = postgres.difference(&sqlite).collect();

    assert!(
        only_in_sqlite.is_empty() && only_in_postgres.is_empty(),
        "SQLite/PostgreSQL schema drift detected (comparing (table, column) name sets at v15).\n\
         Only in SQLite ({} pair(s)): {:?}\n\
         Only in PostgreSQL ({} pair(s)): {:?}\n\
         If a difference is a legitimate dialect artifact, extend POSTGRES_ONLY_TABLES \
         (whole table) with a justification; otherwise fix the lagging adapter.",
        only_in_sqlite.len(),
        only_in_sqlite,
        only_in_postgres.len(),
        only_in_postgres,
    );
}

/// Authoritative SQLite side: run the real migration and introspect the result.
fn sqlite_columns_via_real_migration() -> ColumnSet {
    let mut connection = rusqlite::Connection::open_in_memory().expect("open in-memory sqlite");
    tiamud_persistence_sqlite::migrate(&mut connection).expect("sqlite migrate to v15");

    let tables: Vec<String> = {
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .expect("prepare table query");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query tables");
        rows.map(|row| row.expect("read table name")).collect()
    };

    let mut columns = ColumnSet::new();
    for table in tables {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .expect("prepare pragma");
        let rows = statement
            // table_info column 1 is the column name.
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query columns");
        for row in rows {
            columns.insert((table.clone(), row.expect("read column name")));
        }
    }
    columns
}

/// Static PostgreSQL side: parse `(table, column)` pairs out of the fresh
/// install DDL string. No PostgreSQL server is used or required.
fn postgres_columns_via_static_ddl_parse() -> ColumnSet {
    parse_create_table_columns(tiamud_persistence_postgres::SCHEMA_SQL)
}

/// Extract `(table, column)` pairs from a block of `CREATE TABLE` DDL.
///
/// The parser is paren-depth and single-quote aware so it does not mistake
/// commas or tokens inside `CHECK (...)` clauses, string literals, or default
/// expressions for column boundaries. Table-level constraint clauses
/// (`PRIMARY KEY`, `FOREIGN KEY`, `UNIQUE`, `CHECK`, `CONSTRAINT`, ...) are
/// recognised by their leading keyword and skipped. `CREATE INDEX`,
/// `CREATE UNIQUE INDEX`, `CREATE FUNCTION`, and `CREATE TRIGGER` are never
/// matched because only the literal `CREATE TABLE ` prefix starts a block.
fn parse_create_table_columns(ddl: &str) -> ColumnSet {
    const MARKER: &str = "CREATE TABLE ";
    let mut columns = ColumnSet::new();
    let bytes = ddl.as_bytes();
    let mut search_from = 0usize;

    while let Some(offset) = ddl[search_from..].find(MARKER) {
        let after_marker = search_from + offset + MARKER.len();
        // Table name: identifier up to whitespace or the opening paren. Handles
        // an optional `IF NOT EXISTS` (not used today but cheap to tolerate).
        let mut cursor = after_marker;
        let rest = &ddl[cursor..];
        let rest_trimmed = rest.trim_start_matches(|c: char| c.is_whitespace());
        cursor += rest.len() - rest_trimmed.len();
        let mut remaining = &ddl[cursor..];
        if let Some(stripped) = remaining
            .strip_prefix("IF NOT EXISTS")
            .or_else(|| remaining.strip_prefix("if not exists"))
        {
            let advanced = remaining.len() - stripped.len();
            cursor += advanced;
            let ws = &ddl[cursor..];
            let ws_trimmed = ws.trim_start_matches(|c: char| c.is_whitespace());
            cursor += ws.len() - ws_trimmed.len();
            remaining = &ddl[cursor..];
        }
        let name_end = remaining
            .find(|c: char| c.is_whitespace() || c == '(')
            .map(|i| cursor + i)
            .unwrap_or(ddl.len());
        let table = ddl[cursor..name_end].trim().to_string();

        // Find the opening paren of the column list.
        let Some(open_rel) = ddl[name_end..].find('(') else {
            break;
        };
        let body_start = name_end + open_rel + 1;

        // Walk the body tracking paren depth and single-quote state until the
        // matching close paren returns depth to zero.
        let mut depth = 1i32;
        let mut in_quote = false;
        let mut i = body_start;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if in_quote {
                if c == '\'' {
                    in_quote = false;
                }
            } else {
                match c {
                    '\'' => in_quote = true,
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        let body_end = i;
        let body = &ddl[body_start..body_end];

        for segment in split_top_level_commas(body) {
            if let Some(column) = column_name_of(&segment) {
                columns.insert((table.clone(), column));
            }
        }

        search_from = body_end;
    }

    columns
}

/// Split a `CREATE TABLE` body into its top-level comma-separated items,
/// ignoring commas nested inside parentheses or single-quoted strings.
fn split_top_level_commas(body: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut current = String::new();
    for c in body.chars() {
        if in_quote {
            if c == '\'' {
                in_quote = false;
            }
            current.push(c);
            continue;
        }
        match c {
            '\'' => {
                in_quote = true;
                current.push(c);
            }
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
}

/// Return the column name of a table-body item, or `None` if the item is a
/// table-level constraint rather than a column definition.
fn column_name_of(segment: &str) -> Option<String> {
    const CONSTRAINT_KEYWORDS: &[&str] = &[
        "CONSTRAINT",
        "PRIMARY",
        "FOREIGN",
        "UNIQUE",
        "CHECK",
        "EXCLUDE",
        "LIKE",
    ];
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first: String = trimmed
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '(')
        .collect();
    if first.is_empty() {
        return None;
    }
    if CONSTRAINT_KEYWORDS
        .iter()
        .any(|kw| kw.eq_ignore_ascii_case(&first))
    {
        return None;
    }
    // Strip optional double-quote identifier quoting.
    Some(first.trim_matches('"').to_string())
}

/// Guard tests for the static parser itself, so the parity assertion cannot
/// pass merely because the parser produced garbage or silently agreed with an
/// empty set.
#[test]
fn parser_extracts_columns_and_skips_table_level_constraints() {
    let ddl = r#"
CREATE TABLE demo (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 64),
    kind TEXT NOT NULL DEFAULT 'character'
        CONSTRAINT demo_kind_check CHECK (kind IN ('a', 'b', 'c')),
    payload TEXT NOT NULL
        CHECK (
            (kind = 'a' AND payload IS NULL)
            OR (kind = 'b' AND payload IS NOT NULL)
        ),
    PRIMARY KEY (id, name),
    UNIQUE (name),
    CHECK (id <> 0)
);
CREATE INDEX demo_name_idx ON demo (name);
CREATE UNIQUE INDEX demo_id_idx ON demo (id);
"#;
    let columns = parse_create_table_columns(ddl);
    let names: BTreeSet<String> = columns
        .iter()
        .filter(|(table, _)| table == "demo")
        .map(|(_, column)| column.clone())
        .collect();

    let expected: BTreeSet<String> = ["id", "name", "kind", "payload"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        names, expected,
        "parser must capture exactly the columns and skip PRIMARY KEY/UNIQUE/CHECK table clauses"
    );
    // The multi-line CHECK references `payload` and `kind` inside its body; those
    // must not leak in as phantom columns, and index DDL must be ignored.
    assert!(!names.contains("PRIMARY"));
    assert!(!names.contains("CHECK"));
}

#[test]
fn parser_finds_a_deliberately_divergent_column() {
    let with_extra = "CREATE TABLE t (a BIGINT, b TEXT, c TEXT);";
    let without_extra = "CREATE TABLE t (a BIGINT, b TEXT);";
    let full = parse_create_table_columns(with_extra);
    let partial = parse_create_table_columns(without_extra);
    let difference: Vec<_> = full.difference(&partial).collect();
    assert_eq!(difference, vec![&("t".to_string(), "c".to_string())]);
}
