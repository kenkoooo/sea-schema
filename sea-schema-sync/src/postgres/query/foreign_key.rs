use super::SchemaQueryBuilder;
use crate::rusqlite_types::RusqliteRow;

/// One (local column, referenced column) pair of a foreign key, read from
/// `pg_catalog.pg_constraint`.
#[derive(Debug, Default)]
pub struct ForeignKeyQueryResult {
    pub constraint_name: String,
    pub column_name: String,
    pub foreign_table_name: String,
    pub foreign_column_name: String,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

impl SchemaQueryBuilder {
    /// Build a query that reads foreign key information from
    /// `pg_catalog.pg_constraint`.
    ///
    /// `information_schema` cannot describe a foreign key that references a bare
    /// `UNIQUE INDEX` (rather than a named `UNIQUE` / `PRIMARY KEY` constraint):
    /// it does not expose the referenced constraint, so the local and referenced
    /// columns cannot be correlated and discovery falls back to their cartesian
    /// product, emitting duplicated columns.
    ///
    /// `pg_constraint` instead stores the local and referenced columns as the
    /// parallel arrays `conkey` / `confkey`, so unnesting them together yields
    /// the intended 1:1 column pairing in order, regardless of how the
    /// referenced uniqueness is enforced. The single-character `confupdtype` /
    /// `confdeltype` codes are mapped to the same wording `information_schema`
    /// uses so the rows parse identically.
    ///
    /// This is a raw query (executed via [`Connection::query_all_raw`]) because
    /// it relies on `unnest(..) WITH ORDINALITY`, which the query builder cannot
    /// express.
    ///
    /// [`Connection::query_all_raw`]: crate::Connection::query_all_raw
    pub fn query_table_references(&self, schema: &str, table: &str) -> String {
        // Escape single quotes so the names are embedded as safe string literals.
        let schema = schema.replace('\'', "''");
        let table = table.replace('\'', "''");
        format!(
            "SELECT \
                con.conname AS constraint_name, \
                col.attname AS column_name, \
                ref_tbl.relname AS foreign_table_name, \
                ref_col.attname AS foreign_column_name, \
                CASE con.confupdtype \
                    WHEN 'a' THEN 'NO ACTION' \
                    WHEN 'r' THEN 'RESTRICT' \
                    WHEN 'c' THEN 'CASCADE' \
                    WHEN 'n' THEN 'SET NULL' \
                    WHEN 'd' THEN 'SET DEFAULT' \
                END AS on_update, \
                CASE con.confdeltype \
                    WHEN 'a' THEN 'NO ACTION' \
                    WHEN 'r' THEN 'RESTRICT' \
                    WHEN 'c' THEN 'CASCADE' \
                    WHEN 'n' THEN 'SET NULL' \
                    WHEN 'd' THEN 'SET DEFAULT' \
                END AS on_delete \
            FROM pg_catalog.pg_constraint con \
            JOIN pg_catalog.pg_class tbl ON tbl.oid = con.conrelid \
            JOIN pg_catalog.pg_namespace ns ON ns.oid = tbl.relnamespace \
            JOIN pg_catalog.pg_class ref_tbl ON ref_tbl.oid = con.confrelid \
            CROSS JOIN LATERAL unnest(con.conkey, con.confkey) \
                WITH ORDINALITY AS keys(conkey, confkey, ord) \
            JOIN pg_catalog.pg_attribute col \
                ON col.attrelid = con.conrelid AND col.attnum = keys.conkey \
            JOIN pg_catalog.pg_attribute ref_col \
                ON ref_col.attrelid = con.confrelid AND ref_col.attnum = keys.confkey \
            WHERE con.contype = 'f' \
                AND ns.nspname = '{schema}' \
                AND tbl.relname = '{table}' \
            ORDER BY con.conname, keys.ord"
        )
    }
}

#[cfg(feature = "sqlx-postgres")]
impl From<RusqliteRow> for ForeignKeyQueryResult {
    fn from(row: RusqliteRow) -> Self {
        use crate::rusqlite_types::Row;
        let row = row.postgres();
        Self {
            constraint_name: row.get(0),
            column_name: row.get(1),
            foreign_table_name: row.get(2),
            foreign_column_name: row.get(3),
            on_update: row.get(4),
            on_delete: row.get(5),
        }
    }
}

#[cfg(not(feature = "sqlx-postgres"))]
impl From<RusqliteRow> for ForeignKeyQueryResult {
    fn from(_: RusqliteRow) -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_targets_pg_constraint_for_the_given_schema_and_table() {
        let sql = SchemaQueryBuilder::default().query_table_references("public", "second");

        assert!(sql.contains("FROM pg_catalog.pg_constraint con"));
        assert!(sql.contains("unnest(con.conkey, con.confkey)"));
        assert!(sql.contains("WITH ORDINALITY"));
        assert!(sql.contains("con.contype = 'f'"));
        assert!(sql.contains("ns.nspname = 'public'"));
        assert!(sql.contains("tbl.relname = 'second'"));
        assert!(sql.contains("ORDER BY con.conname, keys.ord"));
    }

    #[test]
    fn query_escapes_single_quotes_in_identifiers() {
        let sql = SchemaQueryBuilder::default().query_table_references("sch'ema", "ta'ble");

        assert!(sql.contains("ns.nspname = 'sch''ema'"));
        assert!(sql.contains("tbl.relname = 'ta''ble'"));
    }
}
