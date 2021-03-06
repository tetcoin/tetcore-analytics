// Copyright 2019 Parity Technologies (UK) Ltd.
// This file is part of Tetcore Analytics.

// Tetcore Analytics is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Tetcore Analytics is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Tetcore Analytics.  If not, see <http://www.gnu.org/licenses/>.

pub mod benchmarks;
pub mod filters;
pub mod models;
pub mod nodes;
pub mod peer_data;
pub mod reputation;
pub mod stats;

use actix::prelude::*;
use diesel;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool, PoolError};
use diesel::result::QueryResult;
use diesel::RunQueryDsl;

use self::models::{NewPeerConnection, NewTetcoreLog, PeerConnection};
use crate::{DATABASE_URL, DB_POOL_SIZE};

pub const RECORD_LIMIT: i32 = 10_000;

pub struct DbExecutor {
    pool: Pool<ConnectionManager<PgConnection>>,
}

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

impl DbExecutor {
    // Execute query, log error if any and return result.
    fn with_connection<F, R>(&self, f: F) -> Result<R, PoolError>
    where
        F: FnOnce(&PgConnection) -> R,
    {
        let result = self.pool.get().map(|conn| f(&conn));
        if let Err(e) = &result {
            error!("Couldn't get DB connection from pool: {}", e);
        }
        result
    }

    pub fn new(pool: Pool<ConnectionManager<PgConnection>>) -> Self {
        DbExecutor { pool }
    }
}

impl Message for NewPeerConnection {
    type Result = Result<PeerConnection, String>;
}

impl Handler<NewPeerConnection> for DbExecutor {
    type Result = Result<PeerConnection, String>;

    fn handle(&mut self, msg: NewPeerConnection, _: &mut Self::Context) -> Self::Result {
        use crate::schema::peer_connections;
        #[allow(unused_imports)]
        use crate::schema::peer_connections::dsl::*;
        let pc: Result<Result<PeerConnection, _>, _> = self.with_connection(|conn| {
            let result: QueryResult<PeerConnection> = diesel::insert_into(peer_connections::table)
                .values(&msg)
                .get_result(conn);
            result
        });
        if let Ok(pcr) = pc {
            if let Ok(p) = pcr {
                return Ok(p);
            }
        };
        Err(format!(
            "Error inserting PeerConnection, for ip: {}",
            msg.ip_addr
        ))
    }
}

impl Message for PeerConnection {
    type Result = Result<(), String>;
}

impl Handler<PeerConnection> for DbExecutor {
    type Result = Result<(), String>;

    fn handle(&mut self, msg: PeerConnection, _: &mut Self::Context) -> Self::Result {
        //use crate::schema::peer_connections;
        #[allow(unused_imports)]
        use crate::schema::peer_connections::dsl::*;
        let msg_id = msg.id;
        let result = self.with_connection(|conn| {
            diesel::update(peer_connections.filter(id.eq(msg.id)))
                .set((
                    peer_id.eq(msg.peer_id),
                    ip_addr.eq(msg.ip_addr),
                    name.eq(msg.name),
                    chain.eq(msg.chain),
                    version.eq(msg.version),
                    authority.eq(msg.authority),
                    startup_time.eq(msg.startup_time),
                    implementation.eq(msg.implementation),
                ))
                .execute(conn)
        });
        if let Ok(ir) = result {
            if ir.is_ok() {
                return Ok(());
            }
        };
        Err(format!("Error updating PeerConnection, id: {}", msg_id))
    }
}

pub struct LogBatch(pub Vec<NewTetcoreLog>);

impl Message for LogBatch {
    type Result = Result<(), &'static str>;
}

impl Handler<LogBatch> for DbExecutor {
    type Result = Result<(), &'static str>;

    fn handle(&mut self, msg: LogBatch, _: &mut Self::Context) -> Self::Result {
        use crate::schema::tetcore_logs;
        #[allow(unused_imports)]
        use crate::schema::tetcore_logs::dsl::*;
        let _ = self.with_connection(|conn| {
            match diesel::insert_into(tetcore_logs::table)
                .values(msg.0)
                .execute(conn)
            {
                Err(e) => error!("Error inserting logs: {:?}", e),
                Ok(n) => debug!("Inserted {} tetcore_logs", n),
            }
        });
        Ok(())
    }
}

#[derive(Clone)]
pub struct PurgeLogs {
    pub hours_valid: u32,
}

impl Message for PurgeLogs {
    type Result = Result<(), &'static str>;
}

impl Handler<PurgeLogs> for DbExecutor {
    type Result = Result<(), &'static str>;

    fn handle(&mut self, msg: PurgeLogs, _: &mut Self::Context) -> Self::Result {
        let _ = self.with_connection(|conn| {
            let query = format!(
                "DELETE FROM tetcore_logs \
                 USING peer_connections \
                 WHERE peer_connections.id = peer_connection_id \
                 AND audit = false \
                 AND tetcore_logs.created_at < now() - {} * interval '1 hour'",
                msg.hours_valid
            );
            info!("Cleaning up database - deleting old log messages");
            match diesel::sql_query(query).execute(conn) {
                Err(e) => error!("Error purging expired logs: {:?}", e),
                Ok(n) => info!("Purged {} records from database", n),
            }
        });
        let _ = self.with_connection(|conn| {
            let query = "DELETE FROM peer_connections \
                 WHERE id NOT IN \
                 (SELECT DISTINCT peer_connection_id FROM tetcore_logs)";
            info!("Cleaning up database - deleting unreferenced peer_connections");
            match diesel::sql_query(query).execute(conn) {
                Err(e) => error!("Error purging expired peer_connections: {:?}", e),
                Ok(n) => info!("Purged {} records from database", n),
            }
        });
        Ok(())
    }
}

pub fn create_pool() -> Pool<ConnectionManager<PgConnection>> {
    let manager = ConnectionManager::new(DATABASE_URL.to_string());
    let pool = Pool::builder()
        .max_size(*DB_POOL_SIZE)
        .build(manager)
        .expect("Failed to create pool");
    info!("Database pool created with {} connections", *DB_POOL_SIZE);
    pool
}
