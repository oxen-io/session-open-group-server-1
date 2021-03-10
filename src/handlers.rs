use rusqlite::params;
use warp::{Rejection, http::StatusCode};

use super::models;
use super::storage;

/// Inserts the given `message` into the database if it's valid.
pub async fn insert_message(mut message: models::Message, db_pool: storage::DatabaseConnectionPool) -> Result<impl warp::Reply, Rejection> {
    // Validate the message
    if !message.is_valid() { return Err(warp::reject::custom(models::ValidationError)); }
    // Get a database connection
    let db_conn = storage::get_db_conn(&db_pool)?;
    // Insert the message
    match db_conn.execute(
        "INSERT INTO messages (text) VALUES (?1)",
        params![message.text],
    ) {
        Ok(_) => {
            let id = db_conn.last_insert_rowid(); // TODO: Is there a risk of the `execute()` above and this call not being sync?
            message.server_id = Some(id);
            return Ok(warp::reply::json(&message));
        }
        Err(e) => {
            println!("Couldn't insert message due to error: {:?}.", e);
            return Err(warp::reject::custom(storage::DatabaseError)); 
        }
    }
}

/// Returns the last `options.limit` messages from the database.
pub async fn get_messages(options: models::QueryOptions, db_pool: storage::DatabaseConnectionPool) -> Result<impl warp::Reply, Rejection> {
    // Get a database connection
    let db_conn = storage::get_db_conn(&db_pool)?;
    // Unwrap parameters
    let from_server_id = options.from_server_id.unwrap_or(0);
    let limit = options.limit.unwrap_or(256); // Never return more than 256 messages at once
    // Query the database
    let raw_query: &str;
    if options.from_server_id.is_some() {
        raw_query = "SELECT id, text FROM messages WHERE rowid > (?1) LIMIT (?2)";
    } else {
        raw_query = "SELECT id, text FROM messages ORDER BY rowid DESC LIMIT (?2)";
    }
    let mut query = match db_conn.prepare(&raw_query) {
        Ok(query) => query,
        Err(e) => { 
            println!("Couldn't create database query due to error: {:?}.", e);
            return Err(warp::reject::custom(storage::DatabaseError));
        }
    };
    let rows = match query.query_map(params![from_server_id, limit], |row| {
        Ok(models::Message { server_id : row.get(0)?, text : row.get(1)? })
    }) {
        Ok(rows) => rows,
        Err(e) => {
            println!("Couldn't query database due to error: {:?}.", e);
            return Err(warp::reject::custom(storage::DatabaseError));
        }
    };
    // FIXME: It'd be cleaner to do the below using `collect()`, but the compiler has trouble
    // inferring the item type of `rows` in that case.
    let mut messages: Vec<models::Message> = Vec::new();
    for row in rows {
        match row {
            Ok(message) => messages.push(message),
            Err(e) => {
                println!("Excluding message from response due to database error: {:?}.", e);
                continue;
            }
        }
    }
    // Return the messages
    return Ok(warp::reply::json(&messages));
}

/// Deletes the message with the given `row_id` from the database, if it's present.
pub async fn delete_message(row_id: i64, db_pool: storage::DatabaseConnectionPool) -> Result<impl warp::Reply, Rejection> {
    // Get a database connection and open a transaction
    let db_conn = storage::get_db_conn(&db_pool)?;
    let tx = storage::get_tx(&db_conn)?;
    // Delete the message if it's present
    let changed_row_count = storage::execute("DELETE FROM messages WHERE rowid = (?1)", params![row_id], &db_conn)?;
    // Update the deletions table if needed
    if changed_row_count > 0 {
        storage::execute("INSERT INTO deletions (id) VALUES (?1)", params![row_id], &db_conn);
    }
    // Commit
    tx.commit();
    // Return
    return Ok(StatusCode::OK);
}