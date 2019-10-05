use postgres::{Client, Config, NoTls, Row};
use serde::Serialize;
use serde_json::Value;

use crate::errors::UserError;
use crate::guards::Permission;
use crate::settings;
use crate::utils;

pub struct Database {
    conn: Client,
}

#[derive(Debug, Serialize)]
pub struct Token {
    pub id: i32,
    pub token: String,
    pub permissions: Permission,
    pub userid: i32,
}

#[derive(Debug, Serialize)]
pub struct Ban {
    pub id: i32,
    pub reason: String,
    pub date: chrono::NaiveDateTime,
}

impl Token {
    pub fn json(&self) -> Result<Value, UserError> {
        Ok(serde_json::to_value(&self)?)
    }
}

impl Database {
    pub fn new() -> Result<Database, postgres::Error> {
        debug!(utils::LOGGER, "Connecting to database";
         "host" => &settings::ENV.database.host,
         "port" => settings::ENV.database.port,
         "name" => &settings::ENV.database.name,
         "username" => &settings::ENV.database.username);
        let conn = Config::new()
            .host(&settings::ENV.database.host)
            .port(settings::ENV.database.port)
            .dbname(&settings::ENV.database.name)
            .user(&settings::ENV.database.username)
            .password(&settings::ENV.database.password)
            .application_name(&env!("CARGO_PKG_NAME"))
            .connect(NoTls)?;
        debug!(utils::LOGGER, "Connected to PostgreSQL");
        Ok(Database { conn })
    }

    pub fn setup_tables(&mut self) -> Result<(), postgres::Error> {
        let create_banlist = "
            CREATE TABLE IF NOT EXISTS banlist (
                id integer NOT NULL PRIMARY KEY,
                reason Text NOT NULL,
                date timestamp NOT NULL);";
        debug!(utils::LOGGER, "Creating Table if it doesn't exist";
            "query" => create_banlist, "name" => "banlist");
        self.conn.simple_query(create_banlist)?;

        let permissions_enum = "
            DO $$
            BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'permission') THEN
                    CREATE TYPE permission AS ENUM ('User', 'Admin', 'Root');
                END IF;
            END$$;";
        debug!(utils::LOGGER, "Creating type `permission` if it doesn't exist";
            "query" => permissions_enum, "name" => "banlist");
        self.conn.simple_query(permissions_enum)?;

        let create_tokens = "
            CREATE TABLE IF NOT EXISTS tokens (
                id SERIAL,
                token Text NOT NULL PRIMARY KEY,
                permissions permission NOT NULL,
                userid integer NOT NULL);";

        debug!(utils::LOGGER, "Creating Table if it doesn't exist";
            "query" => create_tokens,  "name" => "tokens");
        self.conn.simple_query(create_tokens)?;
        Ok(())
    }

    //region Tokens
    pub fn create_genesis_token(&mut self) -> Result<(), postgres::Error> {
        let get_genesis_token = "SELECT * FROM tokens WHERE id = 1;";
        debug!(utils::LOGGER, "Checking if Genesis Token exists";
            "query" => get_genesis_token);
        if self.conn.query(get_genesis_token, &[])?.is_empty() {
            info!(utils::LOGGER, "Genesis Token doesn't exist. Creating one";
                "size" => settings::ENV.token_size);
            let token = self.create_token(&Permission::Root, settings::ENV.masterid)?;
            info!(utils::LOGGER, "Created Genesis Token `{}`. Write this down, this will be the only time you see it.", token)
        } else {
            debug!(utils::LOGGER, "Genesis Token exists. Skipping creation.")
        }
        Ok(())
    }

    pub fn get_tokens(&mut self) -> Result<Vec<Token>, postgres::Error> {
        let get_all_tokens = "SELECT * FROM tokens;";
        debug!(utils::LOGGER, "Getting all tokens"; "query" => get_all_tokens);
        let result: Vec<Row> = self.conn.query(get_all_tokens, &[])?;
        Ok(result.into_iter()
                 .map(|row| Token {
                     id: row.get(0),
                     token: row.get(1),
                     permissions: row.get(2),
                     userid: row.get(3),
                 })
                 .collect())
    }

    pub fn get_token_by_id(&mut self, token_id: i32) -> Result<Option<Token>, postgres::Error> {
        let get_token_by_id = "SELECT * FROM tokens WHERE id = $1;";
        debug!(utils::LOGGER, "Getting token by id";
            "id" => token_id, "query" => get_token_by_id);
        let row: Option<Row> = self.conn.query(get_token_by_id, &[&token_id])?.pop();

        Ok(match row {
            Some(token) => Some(Token {
                id: token.get(0),
                token: token.get(1),
                permissions: token.get(2),
                userid: token.get(3),
            }),
            None => None
        })
    }


    pub fn get_token(&mut self, token: String) -> Result<Option<Token>, postgres::Error> {
        let get_token_by_id = "SELECT * FROM tokens WHERE token = $1;";
        debug!(utils::LOGGER, "Getting token"; "query" => get_token_by_id);
        let row: Option<Row> = self.conn.query(get_token_by_id, &[&token])?.pop();

        Ok(match row {
            Some(token) => Some(Token {
                id: token.get(0),
                token: token.get(1),
                permissions: token.get(2),
                userid: token.get(3),
            }),
            None => None
        })
    }

    pub fn create_token(&mut self, permission: &Permission, userid: i32) -> Result<String, postgres::Error> {
        let token = nanoid::generate(settings::ENV.token_size as usize);
        let insert_token = "
            INSERT INTO tokens (
                token,
                permissions,
                userid)
            VALUES ($1, $2, $3);";
        debug!(utils::LOGGER, "Creating Token";
         "query" => insert_token, "permission" => format!("{:?}", permission));
        self.conn.execute(insert_token, &[&token, &permission, &userid])?;
        Ok(token)
    }

    pub fn delete_token_by_id(&mut self, token_id: i32) -> Result<(), postgres::Error> {
        let delete_token_by_id = "DELETE FROM tokens WHERE id = $1;";
        debug!(utils::LOGGER, "Deleting token by id";
            "id" => token_id, "query" => delete_token_by_id);
        self.conn.query(delete_token_by_id, &[&token_id])?;
        Ok(())
    }
    //endregion

    //region Banlist
    pub fn get_bans(&mut self) -> Result<Vec<Ban>, postgres::Error> {
        let get_all_bans = "SELECT * FROM banlist;";
        debug!(utils::LOGGER, "Getting all bans"; "query" => get_all_bans);
        let result: Vec<Row> = self.conn.query(get_all_bans, &[])?;
        Ok(result.into_iter()
                 .map(|row| Ban {
                     id: row.get(0),
                     reason: row.get(1),
                     date: row.get(2),
                 })
                 .collect())
    }

    pub fn add_ban(&mut self, user_id: i32, reason: &String) -> Result<(), postgres::Error> {
        let upsert_ban = "
            INSERT INTO banlist
            VALUES ($1, $2, now())
            ON CONFLICT (id) DO
            UPDATE SET reason=EXCLUDED.reason, date=excluded.date;";
        debug!(utils::LOGGER, "Upserting ban";
            "id" => &user_id, "reason" => &reason, "query" => upsert_ban);
        self.conn.query(upsert_ban, &[&user_id, &reason])?;
        Ok(())
    }

    pub fn get_ban(&mut self, user_id: i32) -> Result<Option<Ban>, postgres::Error> {
        let get_ban = "SELECT * FROM banlist WHERE id = $1;";
        debug!(utils::LOGGER, "Getting token by id";
            "id" => user_id, "query" => get_ban);
        let row: Option<Row> = self.conn.query(get_ban, &[&user_id])?.pop();

        Ok(match row {
            Some(ban) => Some(Ban {
                id: ban.get(0),
                reason: ban.get(1),
                date: ban.get(2),
            }),
            None => None
        })
    }

    pub fn delete_ban(&mut self, user_id: i32) -> Result<(), postgres::Error> {
        let delete_ban = "DELETE FROM banlist WHERE id = $1;";
        debug!(utils::LOGGER, "Deleting ban";
            "id" => user_id, "query" => delete_ban);
        let row: Option<Row> = self.conn.query(delete_ban, &[&user_id])?.pop();

        Ok(())
    }
    //endregion
}

