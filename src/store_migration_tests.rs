use anyhow::Result;
use rusqlite::{Connection, Error, ffi, types::Value};

use super::{Store, notice::tests::archive, schema_version};

#[test]
fn v6_preserves_v5_rows_and_prediction_constraints() -> Result<()> {
    let file = archive()?;
    let db = Connection::open(file.path())?;
    db.execute_batch(include_str!("store_v4.sql"))?;
    db.execute_batch(include_str!("store_v5.sql"))?;
    db.execute(
        "INSERT INTO model_prediction VALUES
         ('lotto645', 2, '2026-11-28', 1, '2026-11-22T03:00:00.000Z', 'recent-bias-v1', ?1, ?2),
         ('pension720', 2, '2026-11-26', 1, '2026-11-20T00:00:00Z', 'recent-bias-v1', ?3, NULL)",
        (
            r#"{ "numbers": [1, 2, 3, 4, 5, 6] }"#,
            r#"[ {"numbers":[1,2,3,4,5,10]}, {"numbers":[1,2,3,4,5,9]},
                 {"numbers":[1,2,3,4,5,8]}, {"numbers":[1,2,3,4,5,7]} ]"#,
            r#"{ "group_no": 2, "digits": "034223" }"#,
        ),
    )?;
    let snapshot = |db: &Connection| -> rusqlite::Result<Vec<Vec<Value>>> {
        let mut statement =
            db.prepare("SELECT * FROM model_prediction ORDER BY series_code, target_round")?;
        let columns = statement.column_count();
        statement
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())?
            .collect()
    };
    let before = snapshot(&db)?;

    let store = Store::open(file.path(), &file.path().with_extension("seed"))?;

    assert_eq!(schema_version(&store.db)?, 6);
    assert_eq!(snapshot(&store.db)?, before);
    for (sql, constraint) in [
        (
            "INSERT INTO model_prediction
             SELECT series_code, target_round, target_date, based_on_round, created_at_utc,
                    'jackpot-joint-v2', ticket_json, alternatives_json
             FROM model_prediction WHERE series_code = 'lotto645'",
            ffi::SQLITE_CONSTRAINT_PRIMARYKEY,
        ),
        (
            "UPDATE model_prediction SET target_round = 3, based_on_round = 2",
            ffi::SQLITE_CONSTRAINT_FOREIGNKEY,
        ),
        (
            "UPDATE model_prediction SET algorithm = 'unknown'",
            ffi::SQLITE_CONSTRAINT_CHECK,
        ),
        (
            "UPDATE model_prediction SET alternatives_json = '[]'",
            ffi::SQLITE_CONSTRAINT_CHECK,
        ),
    ] {
        assert!(
            matches!(store.db.execute(sql, []), Err(Error::SqliteFailure(error, _)) if error.extended_code == constraint),
            "expected constraint {constraint} for: {sql}"
        );
    }
    Ok(())
}
