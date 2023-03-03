use futures_util::TryStreamExt;
use sqlx::{
    sqlite::{
        Sqlite, SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    },
    Row, SqlitePool,
};
use std::{io::Error, path::Path};
use tempfile::TempDir;
use tokio::{fs, select};

#[tokio::main]
async fn main() {
    for run_i in 0..1000 {
        database_commit_consistency(run_i).await;
    }
}

async fn database_commit_consistency(run_i: u32) {
    // Pseudocode:
    // 1. Create two databases `a` and `b`, each having pools of one read and one write connection.
    // 2. Create table `a_table` in `a` and `b_table` in `b`.
    // 3. Write 200 entries into `a_table`
    // 4. In parallel:
    //    a. Do some write operation on `b_table` in a loop.
    //    b. Try to retrieve the latest entry from `a_table`.
    //
    // The step 4a occasionally fails

    let (_a_base_dir, a_pool) = create_temp_db().await.unwrap();
    let (_b_base_dir, b_pool) = create_temp_db().await.unwrap();

    println!("START");
    {
        let mut tx = a_pool.write.begin().await.unwrap();
        sqlx::query("CREATE TABLE a_table (id INTEGER PRIMARY KEY)")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    {
        let mut tx = b_pool.write.begin().await.unwrap();
        sqlx::query("CREATE TABLE b_table (id INTEGER PRIMARY KEY)")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    let highest_id = 200;

    {
        let mut tx = a_pool.write.begin().await.unwrap();

        for i in 0..(highest_id + 1) {
            let _id = sqlx::query(
                "INSERT INTO a_table (id)
                 VALUES (?)
                 RETURNING id",
            )
            .bind(&(i as i64))
            .map(|row: sqlx::sqlite::SqliteRow| row.get::<u32, usize>(0))
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        }

        tx.commit().await.unwrap();
    }

    select! {
        _ = async {
            loop {
                let mut tx = b_pool.write.begin().await.unwrap();
                sqlx::query("DELETE FROM b_table")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
            }
        } => {},
        _ = async {
            let mut conn = a_pool.reads.acquire().await.unwrap();

            let vec: Vec<u32> = sqlx::query("SELECT id FROM a_table WHERE id = ?")
                .bind(&(highest_id as i64))
                .fetch(&mut *conn)
                .map_ok(|row| row.get::<u32, usize>(0))
                .try_collect()
                .await
                .unwrap();

            assert!(!vec.is_empty(), "Failed to retrieve on {}-th iterations", run_i);
        } => {},
    }

    a_pool.close().await.unwrap();
    b_pool.close().await.unwrap();
}

pub(crate) struct Pool {
    // Pool with a single read-only connection.
    reads: SqlitePool,
    // Pool with a single writable connection.
    write: SqlitePool,
}

impl Pool {
    async fn create(connect_options: SqliteConnectOptions) -> Result<Self, sqlx::Error> {
        let common_options = connect_options
            //.journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .pragma("recursive_triggers", "ON");

        let write_options = common_options.clone();
        let write = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .test_before_acquire(false)
            .connect_with(write_options)
            .await?;

        let read_options = common_options.read_only(true);
        let reads = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .test_before_acquire(false)
            .connect_with(read_options)
            .await?;

        Ok(Self { reads, write })
    }

    pub(crate) async fn close(&self) -> Result<(), sqlx::Error> {
        self.write.close().await;
        self.reads.close().await;
        Ok(())
    }
}

async fn create_directory(path: &Path) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).await?
    }

    Ok(())
}

async fn create_pool(path: impl AsRef<Path>) -> Result<Pool, std::io::Error> {
    let path = path.as_ref();

    if fs::metadata(path).await.is_ok() {
        panic!("Already exists");
    }

    create_directory(path).await?;

    let connect_options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);

    let pool = Pool::create(connect_options).await.unwrap();

    Ok(pool)
}

async fn create_temp_db() -> Result<(TempDir, Pool), std::io::Error> {
    let temp_dir = TempDir::new()?;
    let pool = create_pool(temp_dir.path().join("temp.db")).await?;

    Ok((temp_dir, pool))
}
