use std::path::Path;

use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                email TEXT NOT NULL DEFAULT '',
                role TEXT NOT NULL DEFAULT 'user',
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                system_prompt TEXT NOT NULL DEFAULT '',
                llm_provider TEXT NOT NULL DEFAULT '',
                llm_config TEXT NOT NULL DEFAULT '{}',
                tts_provider TEXT NOT NULL DEFAULT '',
                tts_config TEXT NOT NULL DEFAULT '{}',
                asr_provider TEXT NOT NULL DEFAULT '',
                asr_config TEXT NOT NULL DEFAULT '{}',
                vad_provider TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS devices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER,
                device_id TEXT NOT NULL UNIQUE,
                client_id TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL DEFAULT '',
                activated INTEGER NOT NULL DEFAULT 0,
                activation_code TEXT NOT NULL DEFAULT '',
                agent_id INTEGER,
                role_name TEXT NOT NULL DEFAULT 'default',
                online INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_active_at TEXT NOT NULL DEFAULT '',
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY(agent_id) REFERENCES agents(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS activation_challenges (
                device_id TEXT NOT NULL,
                client_id TEXT NOT NULL,
                code TEXT NOT NULL,
                message TEXT NOT NULL DEFAULT '',
                challenge TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (device_id, client_id)
            );

            CREATE TABLE IF NOT EXISTS chat_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                session_id TEXT NOT NULL DEFAULT '',
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS configs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                config_id TEXT NOT NULL,
                provider TEXT NOT NULL DEFAULT '',
                json_data TEXT NOT NULL DEFAULT '{}',
                enabled INTEGER NOT NULL DEFAULT 1,
                is_default INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(type, config_id)
            );

            CREATE TABLE IF NOT EXISTS roles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                prompt TEXT NOT NULL DEFAULT '',
                llm_config_id TEXT,
                tts_config_id TEXT,
                voice TEXT,
                role_type TEXT NOT NULL DEFAULT 'user',
                status TEXT NOT NULL DEFAULT 'active',
                sort_order INTEGER NOT NULL DEFAULT 0,
                is_default INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS api_tokens (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                token_prefix TEXT NOT NULL DEFAULT '',
                token_hash TEXT NOT NULL,
                expires_at TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS knowledge_bases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                provider TEXT NOT NULL DEFAULT 'local',
                config_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS kb_documents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                knowledge_base_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                source_type TEXT NOT NULL DEFAULT 'manual',
                status TEXT NOT NULL DEFAULT 'ready',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(knowledge_base_id) REFERENCES knowledge_bases(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS chat_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT NOT NULL UNIQUE,
                device_id TEXT NOT NULL,
                agent_id INTEGER,
                user_id INTEGER,
                session_id TEXT NOT NULL DEFAULT '',
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                tool_call_id TEXT,
                tool_calls_json TEXT,
                audio_path TEXT,
                audio_format TEXT,
                audio_size INTEGER,
                audio_duration REAL,
                metadata TEXT,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS speaker_groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                agent_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                prompt TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                tts_config_id TEXT,
                voice TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                sample_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS speaker_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                file_name TEXT NOT NULL DEFAULT '',
                duration REAL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(group_id) REFERENCES speaker_groups(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS voice_clones (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                tts_config_id TEXT NOT NULL,
                name TEXT NOT NULL,
                provider TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'pending',
                voice_id TEXT,
                shared_to_all INTEGER NOT NULL DEFAULT 0,
                transcript TEXT NOT NULL DEFAULT '',
                error_message TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS voice_clone_audios (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                clone_id INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                file_name TEXT NOT NULL DEFAULT '',
                transcript_lang TEXT NOT NULL DEFAULT 'zh-CN',
                created_at TEXT NOT NULL,
                FOREIGN KEY(clone_id) REFERENCES voice_clones(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS user_voice_clone_quotas (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                tts_config_id TEXT NOT NULL,
                max_count INTEGER NOT NULL DEFAULT -1,
                used_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(user_id, tts_config_id),
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS voice_clone_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL UNIQUE,
                user_id INTEGER NOT NULL,
                voice_clone_id INTEGER NOT NULL,
                provider TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'queued',
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT NOT NULL DEFAULT '',
                started_at TEXT,
                finished_at TEXT,
                meta_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY(voice_clone_id) REFERENCES voice_clones(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_voice_clone_tasks_clone
                ON voice_clone_tasks(voice_clone_id, created_at DESC);
            ",
        )?;
        let _ = conn.execute("ALTER TABLE devices ADD COLUMN role_id INTEGER", []);
        let _ = conn.execute(
            "ALTER TABLE devices ADD COLUMN last_active_at TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute("ALTER TABLE knowledge_bases ADD COLUMN provider TEXT NOT NULL DEFAULT 'local'", []);
        let _ = conn.execute("ALTER TABLE knowledge_bases ADD COLUMN config_json TEXT NOT NULL DEFAULT '{}'", []);
        let _ = conn.execute("ALTER TABLE knowledge_bases ADD COLUMN status TEXT NOT NULL DEFAULT 'active'", []);
        let _ = conn.execute(
            "ALTER TABLE kb_documents ADD COLUMN external_doc_id TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE kb_documents ADD COLUMN sync_error TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE voice_clone_audios ADD COLUMN transcript_lang TEXT NOT NULL DEFAULT 'zh-CN'",
            [],
        );
        let _ = conn.execute(
            "UPDATE voice_clones SET status = 'active' WHERE status = 'ready'",
            [],
        );
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN extra_json TEXT NOT NULL DEFAULT '{}'", []);
        migrate_devices_nullable_user_id(&conn)?;
        migrate_backfill_chat_message_ownership(&conn)?;
        xiaozhi_logging::ensure_schema(&conn)?;
        purge_ota_probe_devices(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn is_unbound_device(device: &DeviceRow) -> bool {
        device.user_id.is_none()
    }

    pub fn generate_unique_activation_code(&self) -> Result<String> {
        for _ in 0..32 {
            let code = format!("{:06}", uuid::Uuid::new_v4().as_u128() % 1_000_000);
            if self.find_device_by_activation_code(&code)?.is_none() {
                return Ok(code);
            }
        }
        anyhow::bail!("无法生成唯一激活码")
    }

    pub fn find_device_by_activation_code(&self, code: &str) -> Result<Option<DeviceRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices WHERE activation_code = ?1",
            [code],
            map_device_row,
        )
        .optional()
        .map_err(Into::into)
    }

    /// 设备首次 OTA 时自动建档（未绑定用户），返回含激活码的记录
    pub fn ensure_pending_device(&self, device_id: &str, client_id: &str) -> Result<DeviceRow> {
        if xiaozhi_core::constants::ota_test::is_probe_device(device_id) {
            anyhow::bail!("OTA 测试设备不允许建档");
        }
        if let Some(device) = self.find_device_by_device_id(device_id)? {
            return Ok(device);
        }
        let code = self.generate_unique_activation_code()?;
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO devices (user_id, device_id, client_id, name, activation_code, activated, created_at)
             VALUES (NULL,?1,?2,'',?3,0,?4)",
            params![device_id, client_id, code, now],
        )?;
        drop(conn);
        let _ = self.refresh_activation_challenge(device_id, client_id, &code)?;
        self.find_device_by_device_id(device_id)?
            .ok_or_else(|| anyhow::anyhow!("创建设备失败"))
    }

    pub fn refresh_activation_challenge(
        &self,
        device_id: &str,
        client_id: &str,
        code: &str,
    ) -> Result<String> {
        let challenge = uuid::Uuid::new_v4().to_string();
        let expires = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        let message = activation_bind_message(code);
        self.upsert_activation_challenge(device_id, client_id, code, &message, &challenge, &expires)?;
        Ok(challenge)
    }

    pub fn ensure_device_activation_code(&self, device_id: &str) -> Result<String> {
        let Some(device) = self.find_device_by_device_id(device_id)? else {
            anyhow::bail!("设备不存在");
        };
        if !device.activation_code.is_empty() {
            return Ok(device.activation_code);
        }
        let code = self.generate_unique_activation_code()?;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE devices SET activation_code = ?1 WHERE device_id = ?2",
            params![code, device_id],
        )?;
        Ok(code)
    }

    pub fn assign_device_to_user_agent(
        &self,
        device_row_id: i64,
        owner_user_id: i64,
        agent_id: i64,
        nick_name: &str,
    ) -> Result<bool> {
        self.assign_device_to_user(device_row_id, owner_user_id, Some(agent_id), nick_name)
    }

    pub fn create_bound_device(
        &self,
        owner_user_id: i64,
        agent_id: i64,
        device_id: &str,
        client_id: &str,
        nick_name: &str,
    ) -> Result<i64> {
        let code = self.generate_unique_activation_code()?;
        let name = if nick_name.trim().is_empty() {
            device_id.to_string()
        } else {
            nick_name.trim().to_string()
        };
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO devices (user_id, device_id, client_id, name, activation_code, activated, agent_id, created_at)
             VALUES (?1,?2,?3,?4,?5,1,?6,?7)",
            params![
                owner_user_id,
                device_id,
                client_id,
                name,
                code,
                agent_id,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn bind_device_to_agent_by_identifier(
        &self,
        agent_id: i64,
        owner_user_id: i64,
        code: &str,
        device_mac: &str,
        nick_name: &str,
    ) -> Result<DeviceRow, String> {
        let code = code.trim();
        let device_mac = device_mac.trim();
        if code.is_empty() && device_mac.is_empty() {
            return Err("请填写设备验证码或设备 MAC".into());
        }
        if !code.is_empty() && (code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit())) {
            return Err("验证码格式错误".into());
        }

        let device = if !code.is_empty() {
            self.find_device_by_activation_code(code)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| {
                    String::from("验证码无效，请确认设备已联网并在屏幕上显示了该验证码")
                })?
        } else {
            let normalized = normalize_device_mac(device_mac);
            if let Some(device) = self
                .find_device_by_device_id(&normalized)
                .map_err(|e| e.to_string())?
            {
                device
            } else if let Some(device) = self
                .find_device_by_device_id(device_mac)
                .map_err(|e| e.to_string())?
            {
                device
            } else {
                let id = self
                    .create_bound_device(owner_user_id, agent_id, &normalized, "", nick_name)
                    .map_err(|e| e.to_string())?;
                return self
                    .get_device_by_id_admin(id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "设备创建失败".into());
            }
        };

        if let Some(uid) = device.user_id {
            if uid != owner_user_id {
                return Err(if !code.is_empty() {
                    "验证码无效或设备已被其他用户绑定".into()
                } else {
                    "设备 MAC 无效或设备已被其他用户绑定".into()
                });
            }
        }

        let display_name = if nick_name.trim().is_empty() {
            if device.name.trim().is_empty() {
                device.device_id.clone()
            } else {
                device.name.clone()
            }
        } else {
            nick_name.trim().to_string()
        };

        self.assign_device_to_user(device.id, owner_user_id, Some(agent_id), &display_name)
            .map_err(|e| e.to_string())?;
        self.get_device_by_id_admin(device.id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "设备不存在".into())
    }

    pub fn admin_count(&self) -> Result<i64> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin'",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        email: &str,
        role: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO users (username, password_hash, email, role, created_at) VALUES (?1,?2,?3,?4,?5)",
            params![username, password_hash, email, role, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn find_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, username, password_hash, email, role FROM users WHERE username = ?1",
            [username],
            |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    email: r.get(3)?,
                    role: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn find_user_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, username, password_hash, email, role FROM users WHERE id = ?1",
            [id],
            |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    email: r.get(3)?,
                    role: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn count_users(&self) -> Result<i64> {
        let conn = self.conn.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    pub fn count_devices(&self, user_id: Option<i64>) -> Result<i64> {
        let conn = self.conn.lock();
        match user_id {
            Some(uid) => Ok(conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE user_id = ?1",
                [uid],
                |r| r.get(0),
            )?),
            None => Ok(conn.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?),
        }
    }

    pub fn count_online_devices(&self, user_id: Option<i64>) -> Result<i64> {
        let conn = self.conn.lock();
        match user_id {
            Some(uid) => Ok(conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE user_id = ?1 AND online = 1",
                [uid],
                |r| r.get(0),
            )?),
            None => Ok(conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE online = 1",
                [],
                |r| r.get(0),
            )?),
        }
    }

    pub fn count_agents(&self, user_id: Option<i64>) -> Result<i64> {
        let conn = self.conn.lock();
        match user_id {
            Some(uid) => Ok(conn.query_row(
                "SELECT COUNT(*) FROM agents WHERE user_id = ?1",
                [uid],
                |r| r.get(0),
            )?),
            None => Ok(conn.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))?),
        }
    }

    pub fn list_agents(&self, user_id: i64) -> Result<Vec<AgentRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, system_prompt, llm_provider, llm_config, tts_provider, tts_config,
                    asr_provider, asr_config, vad_provider, created_at, COALESCE(extra_json, '{}')
             FROM agents WHERE user_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([user_id], map_agent_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_agent(&self, id: i64, user_id: i64) -> Result<Option<AgentRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, name, system_prompt, llm_provider, llm_config, tts_provider, tts_config,
                    asr_provider, asr_config, vad_provider, created_at, COALESCE(extra_json, '{}')
             FROM agents WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
            map_agent_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn create_agent(&self, user_id: i64, req: &AgentInput) -> Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO agents (user_id, name, system_prompt, llm_provider, llm_config, tts_provider, tts_config,
                                 asr_provider, asr_config, vad_provider, created_at, extra_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                user_id,
                req.resolved_name(),
                req.system_prompt,
                req.llm_provider,
                req.llm_config,
                req.tts_provider,
                req.tts_config,
                req.asr_provider,
                req.asr_config,
                req.vad_provider,
                chrono::Utc::now().to_rfc3339(),
                req.extra_json(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_agent(&self, id: i64, user_id: i64, req: &AgentInput) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE agents SET name=?1, system_prompt=?2, llm_provider=?3, llm_config=?4,
             tts_provider=?5, tts_config=?6, asr_provider=?7, asr_config=?8, vad_provider=?9, extra_json=?10
             WHERE id=?11 AND user_id=?12",
            params![
                req.resolved_name(),
                req.system_prompt,
                req.llm_provider,
                req.llm_config,
                req.tts_provider,
                req.tts_config,
                req.asr_provider,
                req.asr_config,
                req.vad_provider,
                req.extra_json(),
                id,
                user_id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn delete_agent(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM agents WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn update_agent_by_id(&self, id: i64, req: &AgentInput) -> Result<bool> {
        if let Some(agent) = self.get_agent_by_id(id)? {
            return self.update_agent(id, agent.user_id, req);
        }
        Ok(false)
    }

    pub fn delete_agent_by_id(&self, id: i64) -> Result<bool> {
        if let Some(agent) = self.get_agent_by_id(id)? {
            return self.delete_agent(id, agent.user_id);
        }
        Ok(false)
    }

    pub fn update_device_by_id(&self, id: i64, req: &DeviceInput) -> Result<bool> {
        let Some(device) = self.get_device_by_id_admin(id)? else {
            return Ok(false);
        };

        if device.user_id.is_none() {
            return self.update_or_assign_pending_device(id, &device, req);
        }

        let user_id = req.user_id.filter(|u| *u > 0).or(device.user_id);
        let Some(user_id) = user_id else {
            return Ok(false);
        };
        self.update_device(id, user_id, req)
    }

    /// 管理端更新待绑定设备，或在编辑时指定所属用户以完成激活
    fn update_or_assign_pending_device(
        &self,
        id: i64,
        device: &DeviceRow,
        req: &DeviceInput,
    ) -> Result<bool> {
        let device_id = if req.resolved_device_id().is_empty() {
            device.device_id.clone()
        } else {
            req.resolved_device_id()
        };
        let name = if req.name.trim().is_empty() {
            device.name.clone()
        } else {
            req.name.trim().to_string()
        };
        let client_id = if req.client_id.trim().is_empty() {
            device.client_id.clone()
        } else {
            req.client_id.trim().to_string()
        };
        let agent_id = normalize_agent_id(req.agent_id.or(device.agent_id));

        if let Some(user_id) = req.user_id.filter(|u| *u > 0) {
            self.assign_device_to_user(id, user_id, agent_id, &name)?;
            let conn = self.conn.lock();
            let n = conn.execute(
                "UPDATE devices SET device_id = ?1, client_id = ?2 WHERE id = ?3",
                params![device_id, client_id, id],
            )?;
            return Ok(n > 0);
        }

        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET device_id = ?1, client_id = ?2, name = ?3, agent_id = ?4
             WHERE id = ?5 AND user_id IS NULL",
            params![device_id, client_id, name, agent_id, id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_device_by_id(&self, id: i64) -> Result<bool> {
        if let Some(device) = self.get_device_by_id_admin(id)? {
            if let Some(user_id) = device.user_id {
                return self.delete_device(id, user_id);
            }
            let conn = self.conn.lock();
            let n = conn.execute("DELETE FROM devices WHERE id = ?1", [id])?;
            return Ok(n > 0);
        }
        Ok(false)
    }

    pub fn list_pending_devices(&self) -> Result<Vec<DeviceRow>> {
        let probe_id = xiaozhi_core::constants::ota_test::DEVICE_ID;
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices WHERE user_id IS NULL AND LOWER(device_id) != LOWER(?1) ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([probe_id], map_device_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn claim_device_by_code(
        &self,
        owner_user_id: i64,
        code: &str,
        agent_id: Option<i64>,
        nick_name: &str,
    ) -> Result<DeviceRow, String> {
        let code = code.trim();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err("验证码格式错误".into());
        }
        let device = self
            .find_device_by_activation_code(code)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| String::from("验证码无效，请确认与设备屏幕显示一致"))?;

        if let Some(uid) = device.user_id {
            if uid != owner_user_id {
                return Err("该设备已被其他用户绑定".into());
            }
            if device.activated {
                return Ok(device);
            }
        }

        self.assign_device_to_user(device.id, owner_user_id, agent_id, nick_name)
            .map_err(|e| e.to_string())?;
        self.get_device_by_id_admin(device.id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "设备认领失败".into())
    }

    pub fn assign_device_to_user(
        &self,
        device_row_id: i64,
        owner_user_id: i64,
        agent_id: Option<i64>,
        nick_name: &str,
    ) -> Result<bool> {
        let agent_id = normalize_agent_id(agent_id);
        if let Some(agent_id) = agent_id {
            if self.get_agent_by_id(agent_id)?.is_none() {
                anyhow::bail!("智能体不存在，请先创建智能体或不选择智能体");
            }
        }
        let conn = self.conn.lock();
        let n = if nick_name.trim().is_empty() {
            conn.execute(
                "UPDATE devices SET user_id = ?1, agent_id = ?2, activated = 1 WHERE id = ?3",
                params![owner_user_id, agent_id, device_row_id],
            )?
        } else {
            conn.execute(
                "UPDATE devices SET user_id = ?1, agent_id = ?2, activated = 1, name = ?3 WHERE id = ?4",
                params![
                    owner_user_id,
                    agent_id,
                    nick_name.trim(),
                    device_row_id
                ],
            )?
        };
        Ok(n > 0)
    }

    pub fn list_devices(&self, user_id: i64) -> Result<Vec<DeviceRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices WHERE user_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([user_id], map_device_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(filter_list_devices(rows))
    }

    pub fn create_device(&self, user_id: i64, req: &DeviceInput) -> Result<i64> {
        let device_id = req.resolved_device_id();
        if !device_id.is_empty() {
            if let Some(existing) = self.find_device_by_device_id(&device_id)? {
                if existing.user_id.is_none() {
                    self.assign_device_to_user(
                        existing.id,
                        user_id,
                        req.agent_id,
                        &req.resolved_name(),
                    )?;
                    return Ok(existing.id);
                }
                if existing.user_id == Some(user_id) {
                    return Ok(existing.id);
                }
                anyhow::bail!("该设备 MAC 已在系统中注册");
            }
        }

        let code = self.generate_unique_activation_code()?;
        let name = req.resolved_name();
        let agent_id = normalize_agent_id(req.agent_id);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO devices (user_id, device_id, client_id, name, activation_code, activated, agent_id, created_at)
             VALUES (?1,?2,?3,?4,?5,1,?6,?7)",
            params![
                user_id,
                device_id,
                req.client_id,
                name,
                code,
                agent_id,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_device(&self, id: i64, user_id: i64, req: &DeviceInput) -> Result<bool> {
        let conn = self.conn.lock();
        let device_id = if req.resolved_device_id().is_empty() {
            conn.query_row(
                "SELECT device_id FROM devices WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
                |r| r.get::<_, String>(0),
            )?
        } else {
            req.resolved_device_id()
        };
        let name = req.resolved_name();
        let agent_id = normalize_agent_id(req.agent_id);
        let n = conn.execute(
            "UPDATE devices SET device_id=?1, client_id=?2, name=?3, agent_id=?4
             WHERE id=?5 AND user_id=?6",
            params![
                device_id,
                req.client_id,
                name,
                agent_id,
                id,
                user_id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn delete_device(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM devices WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    /// 为每个用户确保存在默认 Web 模拟设备（无硬件也可调试）。
    pub fn ensure_web_simulator_device(&self, user_id: i64) -> Result<DeviceRow> {
        let device_id = xiaozhi_core::constants::simulator::web_sim_device_id(user_id);
        if let Some(existing) = self.find_device_by_device_id(&device_id)? {
            return Ok(existing);
        }
        let input = DeviceInput {
            device_id: device_id.clone(),
            client_id: format!("web-sim-client-{user_id}"),
            name: "Web 模拟设备".to_string(),
            agent_id: None,
            user_id: Some(user_id),
        };
        self.create_device(user_id, &input)?;
        self.find_device_by_device_id(&device_id)?
            .ok_or_else(|| anyhow::anyhow!("创建默认模拟设备失败"))
    }

    pub fn find_device_by_device_id(&self, device_id: &str) -> Result<Option<DeviceRow>> {
        let conn = self.conn.lock();
        let normalized = normalize_device_mac(device_id);
        conn.query_row(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices
             WHERE lower(replace(device_id, '-', ':')) = lower(replace(?1, '-', ':'))",
            [normalized.as_str()],
            map_device_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_device_presence(&self, device_id: &str, online: bool) -> Result<bool> {
        let normalized = normalize_device_mac(device_id);
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        let n = if online {
            conn.execute(
                "UPDATE devices SET online = 1, last_active_at = ?1
                 WHERE lower(replace(device_id, '-', ':')) = lower(replace(?2, '-', ':'))",
                params![now, normalized],
            )?
        } else {
            conn.execute(
                "UPDATE devices SET online = 0
                 WHERE lower(replace(device_id, '-', ':')) = lower(replace(?1, '-', ':'))",
                [normalized],
            )?
        };
        Ok(n > 0)
    }

    pub fn touch_device_last_active(&self, device_id: &str) -> Result<bool> {
        let normalized = normalize_device_mac(device_id);
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET last_active_at = ?1
             WHERE lower(replace(device_id, '-', ':')) = lower(replace(?2, '-', ':'))",
            params![now, normalized],
        )?;
        Ok(n > 0)
    }

    /// 设备离线：与 Go manager `handleDeviceInactiveRequest` 一致清空活跃时间，并同步 `online=0` 供 Rust 前端判断。
    pub fn set_device_inactive(&self, device_id: &str) -> Result<bool> {
        let normalized = normalize_device_mac(device_id);
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET online = 0, last_active_at = ''
             WHERE lower(replace(device_id, '-', ':')) = lower(replace(?1, '-', ':'))",
            [normalized],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_activation_challenge(
        &self,
        device_id: &str,
        client_id: &str,
        code: &str,
        message: &str,
        challenge: &str,
        expires_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO activation_challenges (device_id, client_id, code, message, challenge, expires_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(device_id, client_id) DO UPDATE SET
               code=excluded.code, message=excluded.message, challenge=excluded.challenge, expires_at=excluded.expires_at",
            params![device_id, client_id, code, message, challenge, expires_at],
        )?;
        Ok(())
    }

    pub fn get_activation_challenge(
        &self,
        device_id: &str,
        client_id: &str,
    ) -> Result<Option<(String, String, String, String)>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT code, message, challenge, expires_at FROM activation_challenges
             WHERE device_id = ?1 AND client_id = ?2",
            params![device_id, client_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn activate_device(&self, device_id: &str, client_id: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET activated = 1,
                client_id = CASE WHEN client_id IS NULL OR trim(client_id) = '' THEN ?2 ELSE client_id END
             WHERE lower(replace(device_id, '-', ':')) = lower(replace(?1, '-', ':'))
               AND (client_id = ?2 OR client_id IS NULL OR trim(client_id) = '')",
            params![device_id, client_id],
        )?;
        Ok(n > 0)
    }

    /// 已激活设备重新配网后同步 client_id（避免激活/HMAC 循环）
    pub fn sync_activated_client_id(&self, device_id: &str, client_id: &str) -> Result<()> {
        if client_id.trim().is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE devices SET client_id = ?2
             WHERE lower(replace(device_id, '-', ':')) = lower(replace(?1, '-', ':'))
               AND activated = 1",
            params![device_id, client_id],
        )?;
        Ok(())
    }

    pub fn save_chat(&self, device_id: &str, session_id: &str, role: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO chat_history (device_id, session_id, role, content, created_at) VALUES (?1,?2,?3,?4,?5)",
            params![device_id, session_id, role, content, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn count_devices_by_agent(&self, agent_id: i64) -> Result<i64> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM devices WHERE agent_id = ?1",
            [agent_id],
            |r| r.get(0),
        )?)
    }

    pub fn list_configs(&self, config_type: &str) -> Result<Vec<ConfigRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, type, name, config_id, provider, json_data, enabled, is_default, created_at, updated_at
             FROM configs WHERE type = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map([config_type], |r| {
                Ok(ConfigRow {
                    id: r.get(0)?,
                    r#type: r.get(1)?,
                    name: r.get(2)?,
                    config_id: r.get(3)?,
                    provider: r.get(4)?,
                    json_data: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    is_default: r.get::<_, i64>(7)? != 0,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn find_config_by_type_and_id(
        &self,
        config_type: &str,
        config_id: &str,
    ) -> Result<Option<ConfigRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, type, name, config_id, provider, json_data, enabled, is_default, created_at, updated_at
             FROM configs WHERE type = ?1 AND config_id = ?2 LIMIT 1",
            params![config_type, config_id],
            |r| {
                Ok(ConfigRow {
                    id: r.get(0)?,
                    r#type: r.get(1)?,
                    name: r.get(2)?,
                    config_id: r.get(3)?,
                    provider: r.get(4)?,
                    json_data: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    is_default: r.get::<_, i64>(7)? != 0,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_config(&self, id: i64) -> Result<Option<ConfigRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, type, name, config_id, provider, json_data, enabled, is_default, created_at, updated_at
             FROM configs WHERE id = ?1",
            [id],
            |r| {
                Ok(ConfigRow {
                    id: r.get(0)?,
                    r#type: r.get(1)?,
                    name: r.get(2)?,
                    config_id: r.get(3)?,
                    provider: r.get(4)?,
                    json_data: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    is_default: r.get::<_, i64>(7)? != 0,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn create_config(&self, input: &ConfigInput) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        if input.is_default {
            conn.execute(
                "UPDATE configs SET is_default = 0 WHERE type = ?1",
                [&input.r#type],
            )?;
        }
        conn.execute(
            "INSERT INTO configs (type, name, config_id, provider, json_data, enabled, is_default, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                input.r#type,
                input.name,
                input.config_id,
                input.provider,
                input.json_data,
                input.enabled as i64,
                input.is_default as i64,
                now,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_config(&self, id: i64, input: &ConfigInput) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        if input.is_default {
            conn.execute(
                "UPDATE configs SET is_default = 0 WHERE type = ?1",
                [&input.r#type],
            )?;
        }
        let n = conn.execute(
            "UPDATE configs SET name=?1, config_id=?2, provider=?3, json_data=?4, enabled=?5, is_default=?6, updated_at=?7
             WHERE id=?8",
            params![
                input.name,
                input.config_id,
                input.provider,
                input.json_data,
                input.enabled as i64,
                input.is_default as i64,
                now,
                id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn delete_config(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute("DELETE FROM configs WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    pub fn toggle_config(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE configs SET enabled = CASE enabled WHEN 1 THEN 0 ELSE 1 END, updated_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(n > 0)
    }

    pub fn count_configs(&self, config_type: &str) -> Result<i64> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM configs WHERE type = ?1",
            [config_type],
            |r| r.get(0),
        )?)
    }

    pub fn list_users(&self) -> Result<Vec<UserListRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, username, email, role, created_at FROM users
             WHERE username != '__unbound_devices__'
             ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(UserListRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    email: r.get(2)?,
                    role: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_user(&self, id: i64, email: &str, role: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE users SET email = ?1, role = ?2 WHERE id = ?3",
            params![email, role, id],
        )?;
        Ok(n > 0)
    }

    pub fn update_user_password(&self, id: i64, password_hash: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![password_hash, id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_user(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute("DELETE FROM users WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    pub fn list_all_agents(&self) -> Result<Vec<AgentRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, system_prompt, llm_provider, llm_config, tts_provider, tts_config,
                    asr_provider, asr_config, vad_provider, created_at, COALESCE(extra_json, '{}')
             FROM agents ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([], map_agent_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_all_devices(&self) -> Result<Vec<DeviceRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([], map_device_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(filter_list_devices(rows))
    }

    pub fn get_agent_by_id(&self, id: i64) -> Result<Option<AgentRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, name, system_prompt, llm_provider, llm_config, tts_provider, tts_config,
                    asr_provider, asr_config, vad_provider, created_at, COALESCE(extra_json, '{}')
             FROM agents WHERE id = ?1",
            [id],
            map_agent_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_devices_by_agent(&self, agent_id: i64, user_id: i64) -> Result<Vec<DeviceRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices WHERE agent_id = ?1 AND user_id = ?2 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, user_id], map_device_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(filter_list_devices(rows))
    }

    pub fn bind_device_to_agent(&self, agent_id: i64, user_id: i64, device_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET agent_id = ?1 WHERE id = ?2 AND user_id = ?3",
            params![agent_id, device_id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn unbind_device_from_agent(
        &self,
        agent_id: i64,
        user_id: i64,
        device_id: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET agent_id = NULL WHERE id = ?1 AND user_id = ?2 AND agent_id = ?3",
            params![device_id, user_id, agent_id],
        )?;
        Ok(n > 0)
    }

    pub fn get_device_by_id(&self, id: i64, user_id: i64) -> Result<Option<DeviceRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
            map_device_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_device_by_id_admin(&self, id: i64) -> Result<Option<DeviceRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, device_id, client_id, name, activated, activation_code, agent_id, role_name, online, created_at, last_active_at
             FROM devices WHERE id = ?1",
            [id],
            map_device_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_device_role(&self, device_id: i64, role_id: Option<i64>, role_name: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET role_id = ?1, role_name = ?2 WHERE id = ?3",
            params![role_id, role_name, device_id],
        )?;
        Ok(n > 0)
    }

    pub fn clear_device_role(&self, device_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE devices SET role_id = NULL, role_name = 'default' WHERE id = ?1",
            [device_id],
        )?;
        Ok(n > 0)
    }

    pub fn list_roles_for_user(&self, user_id: i64, is_admin: bool) -> Result<(Vec<RoleRow>, Vec<RoleRow>)> {
        let conn = self.conn.lock();
        let mut global_stmt = conn.prepare(
            "SELECT id, user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                    role_type, status, sort_order, is_default, created_at, updated_at
             FROM roles WHERE role_type = 'global' ORDER BY sort_order ASC, id ASC",
        )?;
        let global_roles = global_stmt
            .query_map([], map_role_row)?
            .collect::<Result<Vec<_>, _>>()?;

        let user_roles = if is_admin {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                        role_type, status, sort_order, is_default, created_at, updated_at
                 FROM roles WHERE role_type = 'user' ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map([], map_role_row)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                        role_type, status, sort_order, is_default, created_at, updated_at
                 FROM roles WHERE role_type = 'user' AND user_id = ?1 ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map([user_id], map_role_row)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        Ok((global_roles, user_roles))
    }

    pub fn list_global_roles(&self) -> Result<Vec<RoleRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                    role_type, status, sort_order, is_default, created_at, updated_at
             FROM roles WHERE role_type = 'global' ORDER BY sort_order ASC, id ASC",
        )?;
        let rows = stmt
            .query_map([], map_role_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_role(&self, id: i64) -> Result<Option<RoleRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                    role_type, status, sort_order, is_default, created_at, updated_at
             FROM roles WHERE id = ?1",
            [id],
            map_role_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn create_role(&self, input: &RoleInput) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        if input.is_default && input.role_type == "global" {
            conn.execute(
                "UPDATE roles SET is_default = 0 WHERE role_type = 'global'",
                [],
            )?;
        }
        conn.execute(
            "INSERT INTO roles (user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                                role_type, status, sort_order, is_default, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                input.user_id,
                input.name,
                input.description,
                input.prompt,
                input.llm_config_id,
                input.tts_config_id,
                input.voice,
                input.role_type,
                input.status,
                input.sort_order,
                input.is_default as i64,
                now,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_role(&self, id: i64, input: &RoleInput) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        if input.is_default && input.role_type == "global" {
            conn.execute(
                "UPDATE roles SET is_default = 0 WHERE role_type = 'global'",
                [],
            )?;
        }
        let n = conn.execute(
            "UPDATE roles SET name=?1, description=?2, prompt=?3, llm_config_id=?4, tts_config_id=?5,
             voice=?6, status=?7, sort_order=?8, is_default=?9, updated_at=?10 WHERE id=?11",
            params![
                input.name,
                input.description,
                input.prompt,
                input.llm_config_id,
                input.tts_config_id,
                input.voice,
                input.status,
                input.sort_order,
                input.is_default as i64,
                now,
                id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn toggle_role_status(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE roles SET status = CASE status WHEN 'active' THEN 'inactive' ELSE 'active' END,
             updated_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(n > 0)
    }

    pub fn set_default_global_role(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE roles SET is_default = 0 WHERE role_type = 'global'",
            [],
        )?;
        let n = conn.execute(
            "UPDATE roles SET is_default = 1, updated_at = ?1 WHERE id = ?2 AND role_type = 'global'",
            params![chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_role(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute("DELETE FROM roles WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    pub fn find_role_by_name(&self, user_id: i64, role_name: &str) -> Result<Option<RoleRow>> {
        let conn = self.conn.lock();
        let needle = role_name.to_lowercase();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, description, prompt, llm_config_id, tts_config_id, voice,
                    role_type, status, sort_order, is_default, created_at, updated_at
             FROM roles
             WHERE role_type = 'global' OR (role_type = 'user' AND user_id = ?1)",
        )?;
        let roles = stmt
            .query_map([user_id], map_role_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(roles.into_iter().find(|r| {
            r.name.to_lowercase() == needle
                || r.name.to_lowercase().contains(&needle)
                || needle.contains(&r.name.to_lowercase())
        }))
    }

    pub fn list_knowledge_bases(&self, user_id: i64) -> Result<Vec<KnowledgeBaseRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, description, provider, status, config_json, created_at, updated_at
             FROM knowledge_bases WHERE user_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([user_id], |r| {
                Ok(KnowledgeBaseRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    name: r.get(2)?,
                    description: r.get(3)?,
                    provider: r.get(4)?,
                    status: r.get(5)?,
                    config_json: r.get(6)?,
                    created_at: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn count_kb_documents_by_kb_ids(
        &self,
        kb_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, i64>> {
        let mut counts = std::collections::HashMap::new();
        if kb_ids.is_empty() {
            return Ok(counts);
        }
        let placeholders = kb_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT knowledge_base_id, COUNT(*) FROM kb_documents
             WHERE knowledge_base_id IN ({placeholders}) GROUP BY knowledge_base_id"
        );
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = kb_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), |r| Ok((r.get::<_, i64>(0)?, r.get(1)?)))?;
        for row in rows {
            let (kb_id, count) = row?;
            counts.insert(kb_id, count);
        }
        Ok(counts)
    }

    pub fn get_owned_knowledge_base(
        &self,
        id: i64,
        user_id: i64,
    ) -> Result<Option<KnowledgeBaseRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, name, description, provider, status, config_json, created_at, updated_at
             FROM knowledge_bases WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
            |r| {
                Ok(KnowledgeBaseRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    name: r.get(2)?,
                    description: r.get(3)?,
                    provider: r.get(4)?,
                    status: r.get(5)?,
                    config_json: r.get(6)?,
                    created_at: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_knowledge_base(&self, id: i64) -> Result<Option<KnowledgeBaseDetail>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, name, description, provider, status, config_json FROM knowledge_bases WHERE id = ?1",
            [id],
            |r| {
                Ok(KnowledgeBaseDetail {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    provider: r.get(3)?,
                    status: r.get(4)?,
                    config_json: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_knowledge_base_row_by_id(&self, id: i64) -> Result<Option<KnowledgeBaseRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, name, description, provider, status, config_json, created_at, updated_at
             FROM knowledge_bases WHERE id = ?1",
            [id],
            |r| {
                Ok(KnowledgeBaseRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    name: r.get(2)?,
                    description: r.get(3)?,
                    provider: r.get(4)?,
                    status: r.get(5)?,
                    config_json: r.get(6)?,
                    created_at: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn find_knowledge_search_config(&self, provider: &str) -> Result<Option<ConfigRow>> {
        let provider = provider.trim().to_lowercase();
        if provider.is_empty() {
            return Ok(None);
        }
        let rows = self.list_configs("knowledge_search")?;
        let picked = rows
            .iter()
            .filter(|r| r.enabled && r.provider.to_lowercase() == provider)
            .max_by_key(|r| (r.is_default, r.id));
        Ok(picked.cloned())
    }

    pub fn create_knowledge_base(
        &self,
        user_id: i64,
        name: &str,
        description: &str,
        provider: &str,
        status: &str,
        config_json: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO knowledge_bases (user_id, name, description, provider, status, config_json, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                user_id,
                name,
                description,
                provider,
                status,
                config_json,
                now,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_knowledge_base(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM knowledge_bases WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiTokenRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, token_prefix, expires_at, created_at
             FROM api_tokens WHERE user_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([user_id], |r| {
                Ok(ApiTokenRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    name: r.get(2)?,
                    token_prefix: r.get(3)?,
                    expires_at: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_api_token(
        &self,
        user_id: i64,
        name: &str,
        token_hash: &str,
        token_prefix: &str,
        expires_at: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO api_tokens (user_id, name, token_hash, token_prefix, expires_at, created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                user_id,
                name,
                token_hash,
                token_prefix,
                expires_at,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_api_token(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn verify_api_token(&self, raw_token: &str) -> Result<Option<(i64, String, String)>> {
        if raw_token.is_empty() {
            return Ok(None);
        }
        let prefix: String = raw_token.chars().take(8).collect();
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.token_hash, t.expires_at, u.id, u.username, u.role
             FROM api_tokens t
             JOIN users u ON u.id = t.user_id
             WHERE t.token_prefix = ?1",
        )?;
        let mut rows = stmt.query([prefix])?;
        while let Some(row) = rows.next()? {
            let hash: String = row.get(0)?;
            let expires_at: Option<String> = row.get(1)?;
            let user_id: i64 = row.get(2)?;
            let username: String = row.get(3)?;
            let role: String = row.get(4)?;
            if let Some(exp) = expires_at {
                if !exp.is_empty() {
                    if let Ok(exp_dt) = chrono::DateTime::parse_from_rfc3339(&exp) {
                        if exp_dt < chrono::Utc::now() {
                            continue;
                        }
                    }
                }
            }
            if crate::auth::verify_password(raw_token, &hash) {
                return Ok(Some((user_id, username, role)));
            }
        }
        Ok(None)
    }

    pub fn get_chat_message_by_message_id(
        &self,
        message_id: &str,
        user_id: i64,
    ) -> Result<Option<ChatMessageRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, message_id, device_id, agent_id, user_id, session_id, role, content,
                    tool_call_id, tool_calls_json, audio_path, audio_format, audio_size, audio_duration, metadata, created_at
             FROM chat_messages WHERE message_id = ?1 AND user_id = ?2 AND is_deleted = 0",
            params![message_id, user_id],
            map_chat_message_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn save_chat_message(&self, msg: &ChatMessageInput) -> Result<i64> {
        let conn = self.conn.lock();
        if let Ok(existing) = conn.query_row(
            "SELECT id FROM chat_messages WHERE message_id = ?1",
            [&msg.message_id],
            |r| r.get::<_, i64>(0),
        ) {
            conn.execute(
                "UPDATE chat_messages SET content=?1, role=?2, metadata=?3 WHERE id=?4",
                params![msg.content, msg.role, msg.metadata, existing],
            )?;
            return Ok(existing);
        }
        conn.execute(
            "INSERT INTO chat_messages (message_id, device_id, agent_id, user_id, session_id, role, content,
             tool_call_id, tool_calls_json, metadata, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                msg.message_id,
                msg.device_id,
                msg.agent_id,
                msg.user_id,
                msg.session_id,
                msg.role,
                msg.content,
                msg.tool_call_id,
                msg.tool_calls_json,
                msg.metadata,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_chat_message_audio(
        &self,
        message_id: &str,
        audio_path: &str,
        audio_format: &str,
        audio_size: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE chat_messages SET audio_path=?1, audio_format=?2, audio_size=?3 WHERE message_id=?4",
            params![audio_path, audio_format, audio_size, message_id],
        )?;
        Ok(n > 0)
    }

    pub fn list_chat_messages(
        &self,
        agent_id: i64,
        user_id: i64,
        page: i64,
        page_size: i64,
    ) -> Result<(i64, Vec<ChatMessageRow>)> {
        self.query_chat_messages(ChatMessageQuery {
            user_id,
            agent_id: Some(agent_id),
            device_id: None,
            session_id: None,
            role: None,
            start_date: None,
            end_date: None,
            page: Some(page),
            page_size: Some(page_size),
        })
    }

    pub fn query_chat_messages(
        &self,
        q: ChatMessageQuery,
    ) -> Result<(i64, Vec<ChatMessageRow>)> {
        let conn = self.conn.lock();
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chat_messages
             WHERE is_deleted = 0
               AND (
                 user_id = ?1
                 OR device_id IN (SELECT device_id FROM devices WHERE user_id = ?1)
               )
               AND (?2 IS NULL OR agent_id = ?2
                    OR device_id IN (
                      SELECT device_id FROM devices
                      WHERE user_id = ?1
                        AND (agent_id = ?2 OR agent_id IS NULL)
                    ))
               AND (?3 IS NULL OR role = ?3)
               AND (?4 IS NULL OR device_id = ?4)
               AND (?5 IS NULL OR session_id = ?5)
               AND (?6 IS NULL OR date(created_at) >= date(?6))
               AND (?7 IS NULL OR date(created_at) <= date(?7))",
            params![
                q.user_id,
                q.agent_id,
                q.role,
                q.device_id,
                q.session_id,
                q.start_date,
                q.end_date,
            ],
            |r| r.get(0),
        )?;
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(50).max(1);
        let offset = (page - 1) * page_size;
        let limit = q.page_size.unwrap_or(50).max(1);
        let mut stmt = conn.prepare(
            "SELECT id, message_id, device_id, agent_id, user_id, session_id, role, content,
                    tool_call_id, tool_calls_json, audio_path, audio_format, audio_size, audio_duration, metadata, created_at
             FROM chat_messages
             WHERE is_deleted = 0
               AND (
                 user_id = ?1
                 OR device_id IN (SELECT device_id FROM devices WHERE user_id = ?1)
               )
               AND (?2 IS NULL OR agent_id = ?2
                    OR device_id IN (
                      SELECT device_id FROM devices
                      WHERE user_id = ?1
                        AND (agent_id = ?2 OR agent_id IS NULL)
                    ))
               AND (?3 IS NULL OR role = ?3)
               AND (?4 IS NULL OR device_id = ?4)
               AND (?5 IS NULL OR session_id = ?5)
               AND (?6 IS NULL OR date(created_at) >= date(?6))
               AND (?7 IS NULL OR date(created_at) <= date(?7))
             ORDER BY created_at DESC LIMIT ?8 OFFSET ?9",
        )?;
        let rows = stmt
            .query_map(
                params![
                    q.user_id,
                    q.agent_id,
                    q.role,
                    q.device_id,
                    q.session_id,
                    q.start_date,
                    q.end_date,
                    limit,
                    offset,
                ],
                map_chat_message_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((total, rows))
    }

    pub fn query_chat_sessions(
        &self,
        q: ChatSessionQuery,
    ) -> Result<(i64, Vec<ChatSessionSummaryRow>)> {
        let conn = self.conn.lock();
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (
                SELECT cm.session_id
                FROM chat_messages cm
                WHERE cm.is_deleted = 0
                  AND cm.session_id != ''
                  AND (
                    cm.user_id = ?1
                    OR cm.device_id IN (SELECT device_id FROM devices WHERE user_id = ?1)
                  )
                  AND (?2 IS NULL OR cm.agent_id = ?2
                       OR cm.device_id IN (
                         SELECT device_id FROM devices
                         WHERE user_id = ?1
                           AND (agent_id = ?2 OR agent_id IS NULL)
                       ))
                  AND (?3 IS NULL OR cm.device_id = ?3)
                  AND (?4 IS NULL OR date(cm.created_at) >= date(?4))
                  AND (?5 IS NULL OR date(cm.created_at) <= date(?5))
                GROUP BY cm.session_id
             )",
            params![
                q.user_id,
                q.agent_id,
                q.device_id,
                q.start_date,
                q.end_date,
            ],
            |r| r.get(0),
        )?;
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(20).max(1);
        let offset = (page - 1) * page_size;
        let limit = page_size;
        let mut stmt = conn.prepare(
            "SELECT
                cm.session_id,
                MAX(cm.device_id) AS device_id,
                MAX(cm.agent_id) AS agent_id,
                MAX(cm.user_id) AS user_id,
                COUNT(*) AS message_count,
                SUM(CASE WHEN cm.role = 'user' THEN 1 ELSE 0 END) AS user_message_count,
                MIN(cm.created_at) AS started_at,
                MAX(cm.created_at) AS updated_at,
                COALESCE(
                  (SELECT content FROM chat_messages u
                   WHERE u.session_id = cm.session_id AND u.role = 'user' AND u.is_deleted = 0
                   ORDER BY u.created_at ASC LIMIT 1),
                  (SELECT content FROM chat_messages a
                   WHERE a.session_id = cm.session_id AND a.is_deleted = 0
                   ORDER BY a.created_at ASC LIMIT 1),
                  ''
                ) AS preview,
                COALESCE(
                  (SELECT content FROM chat_messages l
                   WHERE l.session_id = cm.session_id AND l.is_deleted = 0
                   ORDER BY l.created_at DESC LIMIT 1),
                  ''
                ) AS last_preview
             FROM chat_messages cm
             WHERE cm.is_deleted = 0
               AND cm.session_id != ''
               AND (
                 cm.user_id = ?1
                 OR cm.device_id IN (SELECT device_id FROM devices WHERE user_id = ?1)
               )
               AND (?2 IS NULL OR cm.agent_id = ?2
                    OR cm.device_id IN (
                      SELECT device_id FROM devices
                      WHERE user_id = ?1
                        AND (agent_id = ?2 OR agent_id IS NULL)
                    ))
               AND (?3 IS NULL OR cm.device_id = ?3)
               AND (?4 IS NULL OR date(cm.created_at) >= date(?4))
               AND (?5 IS NULL OR date(cm.created_at) <= date(?5))
             GROUP BY cm.session_id
             ORDER BY MAX(cm.created_at) DESC
             LIMIT ?6 OFFSET ?7",
        )?;
        let rows = stmt
            .query_map(
                params![
                    q.user_id,
                    q.agent_id,
                    q.device_id,
                    q.start_date,
                    q.end_date,
                    limit,
                    offset,
                ],
                map_chat_session_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((total, rows))
    }

    pub fn query_chat_sessions_admin(
        &self,
        q: AdminChatSessionQuery,
    ) -> Result<(i64, Vec<ChatSessionSummaryRow>)> {
        let conn = self.conn.lock();
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (
                SELECT cm.session_id
                FROM chat_messages cm
                WHERE cm.is_deleted = 0
                  AND cm.session_id != ''
                  AND (?1 IS NULL OR cm.user_id = ?1
                       OR cm.device_id IN (SELECT device_id FROM devices WHERE user_id = ?1))
                  AND (?2 IS NULL OR cm.agent_id = ?2
                       OR cm.device_id IN (
                         SELECT device_id FROM devices
                         WHERE (?1 IS NULL OR user_id = ?1)
                           AND (agent_id = ?2 OR (?1 IS NOT NULL AND agent_id IS NULL))
                       ))
                  AND (?3 IS NULL OR cm.device_id = ?3)
                  AND (?4 IS NULL OR date(cm.created_at) >= date(?4))
                  AND (?5 IS NULL OR date(cm.created_at) <= date(?5))
                GROUP BY cm.session_id
             )",
            params![
                q.user_id,
                q.agent_id,
                q.device_id,
                q.start_date,
                q.end_date,
            ],
            |r| r.get(0),
        )?;
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(20).max(1);
        let offset = (page - 1) * page_size;
        let limit = page_size;
        let mut stmt = conn.prepare(
            "SELECT
                cm.session_id,
                MAX(cm.device_id) AS device_id,
                MAX(cm.agent_id) AS agent_id,
                MAX(cm.user_id) AS user_id,
                COUNT(*) AS message_count,
                SUM(CASE WHEN cm.role = 'user' THEN 1 ELSE 0 END) AS user_message_count,
                MIN(cm.created_at) AS started_at,
                MAX(cm.created_at) AS updated_at,
                COALESCE(
                  (SELECT content FROM chat_messages u
                   WHERE u.session_id = cm.session_id AND u.role = 'user' AND u.is_deleted = 0
                   ORDER BY u.created_at ASC LIMIT 1),
                  (SELECT content FROM chat_messages a
                   WHERE a.session_id = cm.session_id AND a.is_deleted = 0
                   ORDER BY a.created_at ASC LIMIT 1),
                  ''
                ) AS preview,
                COALESCE(
                  (SELECT content FROM chat_messages l
                   WHERE l.session_id = cm.session_id AND l.is_deleted = 0
                   ORDER BY l.created_at DESC LIMIT 1),
                  ''
                ) AS last_preview
             FROM chat_messages cm
             WHERE cm.is_deleted = 0
               AND cm.session_id != ''
               AND (?1 IS NULL OR cm.user_id = ?1
                    OR cm.device_id IN (SELECT device_id FROM devices WHERE user_id = ?1))
               AND (?2 IS NULL OR cm.agent_id = ?2
                    OR cm.device_id IN (
                      SELECT device_id FROM devices
                      WHERE (?1 IS NULL OR user_id = ?1)
                        AND (agent_id = ?2 OR (?1 IS NOT NULL AND agent_id IS NULL))
                    ))
               AND (?3 IS NULL OR cm.device_id = ?3)
               AND (?4 IS NULL OR date(cm.created_at) >= date(?4))
               AND (?5 IS NULL OR date(cm.created_at) <= date(?5))
             GROUP BY cm.session_id
             ORDER BY MAX(cm.created_at) DESC
             LIMIT ?6 OFFSET ?7",
        )?;
        let rows = stmt
            .query_map(
                params![
                    q.user_id,
                    q.agent_id,
                    q.device_id,
                    q.start_date,
                    q.end_date,
                    limit,
                    offset,
                ],
                map_chat_session_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((total, rows))
    }

    pub fn list_session_dialogue(&self, session_id: &str) -> Result<Vec<ChatMessageRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, message_id, device_id, agent_id, user_id, session_id, role, content,
                    tool_call_id, tool_calls_json, audio_path, audio_format, audio_size, audio_duration, metadata, created_at
             FROM chat_messages
             WHERE is_deleted = 0
               AND session_id = ?1
               AND role IN ('user', 'assistant')
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([session_id], map_chat_message_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn query_chat_messages_admin(
        &self,
        q: AdminChatMessageQuery,
    ) -> Result<(i64, Vec<ChatMessageRow>)> {
        let conn = self.conn.lock();
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chat_messages
             WHERE is_deleted = 0
               AND (?1 IS NULL OR user_id = ?1
                    OR device_id IN (SELECT device_id FROM devices WHERE user_id = ?1))
               AND (?2 IS NULL OR agent_id = ?2
                    OR device_id IN (
                      SELECT device_id FROM devices
                      WHERE (?1 IS NULL OR user_id = ?1)
                        AND (agent_id = ?2 OR (?1 IS NOT NULL AND agent_id IS NULL))
                    ))
               AND (?3 IS NULL OR role = ?3)
               AND (?4 IS NULL OR device_id = ?4)
               AND (?5 IS NULL OR session_id = ?5)
               AND (?6 IS NULL OR date(created_at) >= date(?6))
               AND (?7 IS NULL OR date(created_at) <= date(?7))",
            params![
                q.user_id,
                q.agent_id,
                q.role,
                q.device_id,
                q.session_id,
                q.start_date,
                q.end_date,
            ],
            |r| r.get(0),
        )?;
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(50).max(1);
        let offset = (page - 1) * page_size;
        let limit = q.page_size.unwrap_or(50).max(1);
        let mut stmt = conn.prepare(
            "SELECT id, message_id, device_id, agent_id, user_id, session_id, role, content,
                    tool_call_id, tool_calls_json, audio_path, audio_format, audio_size, audio_duration, metadata, created_at
             FROM chat_messages
             WHERE is_deleted = 0
               AND (?1 IS NULL OR user_id = ?1
                    OR device_id IN (SELECT device_id FROM devices WHERE user_id = ?1))
               AND (?2 IS NULL OR agent_id = ?2
                    OR device_id IN (
                      SELECT device_id FROM devices
                      WHERE (?1 IS NULL OR user_id = ?1)
                        AND (agent_id = ?2 OR (?1 IS NOT NULL AND agent_id IS NULL))
                    ))
               AND (?3 IS NULL OR role = ?3)
               AND (?4 IS NULL OR device_id = ?4)
               AND (?5 IS NULL OR session_id = ?5)
               AND (?6 IS NULL OR date(created_at) >= date(?6))
               AND (?7 IS NULL OR date(created_at) <= date(?7))
             ORDER BY created_at DESC LIMIT ?8 OFFSET ?9",
        )?;
        let rows = stmt
            .query_map(
                params![
                    q.user_id,
                    q.agent_id,
                    q.role,
                    q.device_id,
                    q.session_id,
                    q.start_date,
                    q.end_date,
                    limit,
                    offset,
                ],
                map_chat_message_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((total, rows))
    }

    pub fn delete_chat_message_admin(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE chat_messages SET is_deleted = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(n > 0)
    }

    pub fn export_chat_messages(&self, q: ChatMessageQuery) -> Result<Vec<ChatMessageRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, message_id, device_id, agent_id, user_id, session_id, role, content,
                    tool_call_id, tool_calls_json, audio_path, audio_format, audio_size, audio_duration, metadata, created_at
             FROM chat_messages
             WHERE is_deleted = 0
               AND (
                 user_id = ?1
                 OR device_id IN (SELECT device_id FROM devices WHERE user_id = ?1)
               )
               AND (?2 IS NULL OR agent_id = ?2
                    OR device_id IN (
                      SELECT device_id FROM devices
                      WHERE user_id = ?1
                        AND (agent_id = ?2 OR agent_id IS NULL)
                    ))
               AND (?3 IS NULL OR role = ?3)
               AND (?4 IS NULL OR device_id = ?4)
               AND (?5 IS NULL OR session_id = ?5)
               AND (?6 IS NULL OR date(created_at) >= date(?6))
               AND (?7 IS NULL OR date(created_at) <= date(?7))
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(
                params![
                    q.user_id,
                    q.agent_id,
                    q.role,
                    q.device_id,
                    q.session_id,
                    q.start_date,
                    q.end_date,
                ],
                map_chat_message_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_chat_message(&self, id: i64, user_id: i64) -> Result<Option<ChatMessageRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, message_id, device_id, agent_id, user_id, session_id, role, content,
                    tool_call_id, tool_calls_json, audio_path, audio_format, audio_size, audio_duration, metadata, created_at
             FROM chat_messages WHERE id = ?1 AND user_id = ?2 AND is_deleted = 0",
            params![id, user_id],
            map_chat_message_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_chat_message(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE chat_messages SET is_deleted = 1 WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn list_mcp_service_names(&self) -> Result<Vec<String>> {
        Ok(crate::mcp_imported_merge::list_enabled_global_mcp_service_names(self)
            .unwrap_or_default())
    }

    pub fn list_speaker_groups(&self, user_id: i64, agent_id: Option<i64>) -> Result<Vec<SpeakerGroupRow>> {
        let conn = self.conn.lock();
        if let Some(aid) = agent_id {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, agent_id, name, prompt, description, tts_config_id, voice, status, sample_count, created_at, updated_at
                 FROM speaker_groups WHERE user_id = ?1 AND agent_id = ?2 ORDER BY id DESC",
            )?;
            let rows = stmt
                .query_map(params![user_id, aid], map_speaker_group_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, agent_id, name, prompt, description, tts_config_id, voice, status, sample_count, created_at, updated_at
                 FROM speaker_groups WHERE user_id = ?1 ORDER BY id DESC",
            )?;
            let rows = stmt
                .query_map([user_id], map_speaker_group_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    pub fn speaker_group_name_exists(
        &self,
        user_id: i64,
        name: &str,
        exclude_id: Option<i64>,
    ) -> Result<bool> {
        let conn = self.conn.lock();
        let count: i64 = if let Some(id) = exclude_id {
            conn.query_row(
                "SELECT COUNT(1) FROM speaker_groups WHERE user_id = ?1 AND name = ?2 AND id <> ?3",
                params![user_id, name, id],
                |r| r.get(0),
            )?
        } else {
            conn.query_row(
                "SELECT COUNT(1) FROM speaker_groups WHERE user_id = ?1 AND name = ?2",
                params![user_id, name],
                |r| r.get(0),
            )?
        };
        Ok(count > 0)
    }

    pub fn create_speaker_group(&self, user_id: i64, input: &SpeakerGroupInput) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO speaker_groups (user_id, agent_id, name, prompt, description, tts_config_id, voice, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                user_id,
                input.agent_id,
                input.name,
                input.prompt,
                input.description,
                input.tts_config_id,
                input.voice,
                now,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_speaker_group(&self, id: i64, user_id: i64, input: &SpeakerGroupInput) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE speaker_groups SET agent_id=?1, name=?2, prompt=?3, description=?4, tts_config_id=?5, voice=?6, updated_at=?7
             WHERE id=?8 AND user_id=?9",
            params![
                input.agent_id,
                input.name,
                input.prompt,
                input.description,
                input.tts_config_id,
                input.voice,
                now,
                id,
                user_id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn delete_speaker_group(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM speaker_groups WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn add_speaker_sample(&self, group_id: i64, file_path: &str, file_name: &str) -> Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO speaker_samples (group_id, file_path, file_name, created_at) VALUES (?1,?2,?3,?4)",
            params![group_id, file_path, file_name, chrono::Utc::now().to_rfc3339()],
        )?;
        conn.execute(
            "UPDATE speaker_groups SET sample_count = sample_count + 1, updated_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), group_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_speaker_samples(&self, group_id: i64) -> Result<Vec<SpeakerSampleRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, file_path, file_name, duration, created_at FROM speaker_samples WHERE group_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([group_id], |r| {
                Ok(SpeakerSampleRow {
                    id: r.get(0)?,
                    group_id: r.get(1)?,
                    file_path: r.get(2)?,
                    file_name: r.get(3)?,
                    duration: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_speaker_sample(&self, sample_id: i64) -> Result<Option<SpeakerSampleRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, group_id, file_path, file_name, duration, created_at FROM speaker_samples WHERE id = ?1",
            [sample_id],
            |r| {
                Ok(SpeakerSampleRow {
                    id: r.get(0)?,
                    group_id: r.get(1)?,
                    file_path: r.get(2)?,
                    file_name: r.get(3)?,
                    duration: r.get(4)?,
                    created_at: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_speaker_sample(&self, group_id: i64, sample_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM speaker_samples WHERE id = ?1 AND group_id = ?2",
            params![sample_id, group_id],
        )?;
        if n > 0 {
            let _ = conn.execute(
                "UPDATE speaker_groups SET sample_count = CASE WHEN sample_count > 0 THEN sample_count - 1 ELSE 0 END, updated_at = ?1 WHERE id = ?2",
                params![chrono::Utc::now().to_rfc3339(), group_id],
            );
        }
        Ok(n > 0)
    }

    pub fn list_voice_clones(&self, user_id: i64) -> Result<Vec<VoiceCloneRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, tts_config_id, name, provider, status, voice_id, shared_to_all, transcript, error_message, created_at, updated_at
             FROM voice_clones WHERE user_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([user_id], map_voice_clone_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_admin_shared_voice_clones(
        &self,
        exclude_user_id: i64,
        provider: &str,
        tts_config_id: &str,
    ) -> Result<Vec<VoiceCloneRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT vc.id, vc.user_id, vc.tts_config_id, vc.name, vc.provider, vc.status, vc.voice_id, vc.shared_to_all, vc.transcript, vc.error_message, vc.created_at, vc.updated_at
             FROM voice_clones vc
             JOIN users u ON u.id = vc.user_id
             WHERE vc.user_id <> ?1
               AND vc.provider = ?2
               AND vc.tts_config_id = ?3
               AND vc.status IN ('active', 'ready')
               AND vc.shared_to_all = 1
               AND u.role = 'admin'
             ORDER BY vc.created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![exclude_user_id, provider, tts_config_id], map_voice_clone_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_voice_clone(
        &self,
        user_id: i64,
        input: &VoiceCloneInput,
        provider: &str,
        status: &str,
        voice_id: Option<&str>,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO voice_clones (user_id, tts_config_id, name, provider, status, voice_id, transcript, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                user_id,
                input.tts_config_id,
                input.name,
                provider,
                status,
                voice_id,
                input.transcript,
                now,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn mark_voice_clone_processing(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE voice_clones SET status = 'processing', error_message = NULL, updated_at = ?1
             WHERE id = ?2 AND user_id = ?3",
            params![chrono::Utc::now().to_rfc3339(), id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn finish_voice_clone_success(&self, id: i64, voice_id: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE voice_clones SET status = 'active', voice_id = ?1, error_message = NULL, updated_at = ?2
             WHERE id = ?3",
            params![voice_id, chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(n > 0)
    }

    pub fn finish_voice_clone_failed(&self, id: i64, error: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE voice_clones SET status = 'failed', error_message = ?1, updated_at = ?2
             WHERE id = ?3",
            params![error, chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(n > 0)
    }

    pub fn update_voice_clone(&self, id: i64, user_id: i64, name: Option<&str>, shared: Option<bool>) -> Result<bool> {
        let conn = self.conn.lock();
        let n = if let Some(n) = name {
            conn.execute(
                "UPDATE voice_clones SET name=?1, updated_at=?2 WHERE id=?3 AND user_id=?4",
                params![n, chrono::Utc::now().to_rfc3339(), id, user_id],
            )?
        } else if let Some(s) = shared {
            conn.execute(
                "UPDATE voice_clones SET shared_to_all=?1, updated_at=?2 WHERE id=?3 AND user_id=?4",
                params![s as i64, chrono::Utc::now().to_rfc3339(), id, user_id],
            )?
        } else {
            return Ok(false);
        };
        Ok(n > 0)
    }

    pub fn delete_voice_clone(&self, id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM voice_clones WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        Ok(n > 0)
    }

    pub fn consume_voice_clone_quota(&self, user_id: i64, tts_config_id: &str) -> Result<(), String> {
        let tts_config_id = tts_config_id.trim();
        if tts_config_id.is_empty() {
            return Ok(());
        }
        if let Some(user) = self
            .find_user_by_id(user_id)
            .map_err(|e| e.to_string())?
        {
            if user.role.eq_ignore_ascii_case("admin") {
                return Ok(());
            }
        }
        let conn = self.conn.lock();
        let quota: Option<(i64, i64, i64)> = conn
            .query_row(
                "SELECT id, max_count, used_count FROM user_voice_clone_quotas
                 WHERE user_id = ?1 AND tts_config_id = ?2",
                params![user_id, tts_config_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((quota_id, max_count, _used_count)) = quota else {
            return Err("声音复刻额度不足，请联系管理员分配额度".to_string());
        };
        if max_count < 0 {
            return Ok(());
        }
        let n = conn
            .execute(
                "UPDATE user_voice_clone_quotas SET used_count = used_count + 1, updated_at = ?1
                 WHERE id = ?2 AND max_count >= 0 AND used_count < max_count",
                params![chrono::Utc::now().to_rfc3339(), quota_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("声音复刻额度不足，请联系管理员分配额度".to_string());
        }
        Ok(())
    }

    pub fn count_voice_clones_by_tts_config(&self, user_id: i64) -> Result<Vec<(String, i64)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT tts_config_id, COUNT(1) FROM voice_clones
             WHERE user_id = ?1 AND status != 'deleted' GROUP BY tts_config_id",
        )?;
        let rows = stmt
            .query_map([user_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_voice_clone_quotas(&self, user_id: i64) -> Result<Vec<VoiceCloneQuotaRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, tts_config_id, max_count, used_count, created_at, updated_at
             FROM user_voice_clone_quotas WHERE user_id = ?1",
        )?;
        let rows = stmt
            .query_map([user_id], |r| {
                Ok(VoiceCloneQuotaRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    tts_config_id: r.get(2)?,
                    max_count: r.get(3)?,
                    used_count: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn upsert_voice_clone_quota(
        &self,
        user_id: i64,
        tts_config_id: &str,
        max_count: i64,
        used_count: i64,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        let updated = conn.execute(
            "UPDATE user_voice_clone_quotas SET max_count = ?1, used_count = ?2, updated_at = ?3
             WHERE user_id = ?4 AND tts_config_id = ?5",
            params![max_count, used_count, now, user_id, tts_config_id],
        )?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO user_voice_clone_quotas (user_id, tts_config_id, max_count, used_count, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![user_id, tts_config_id, max_count, used_count, now, now],
            )?;
        }
        Ok(())
    }

    pub fn delete_voice_clone_quota(&self, user_id: i64, tts_config_id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM user_voice_clone_quotas WHERE user_id = ?1 AND tts_config_id = ?2",
            params![user_id, tts_config_id],
        )?;
        Ok(())
    }

    pub fn add_voice_clone_audio(
        &self,
        clone_id: i64,
        file_path: &str,
        file_name: &str,
        transcript_lang: &str,
    ) -> Result<i64> {
        let lang = if transcript_lang.trim().is_empty() {
            "zh-CN"
        } else {
            transcript_lang.trim()
        };
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO voice_clone_audios (clone_id, file_path, file_name, transcript_lang, created_at) VALUES (?1,?2,?3,?4,?5)",
            params![clone_id, file_path, file_name, lang, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_voice_clone_audios(&self, clone_id: i64) -> Result<Vec<VoiceCloneAudioRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, clone_id, file_path, file_name, transcript_lang, created_at FROM voice_clone_audios WHERE clone_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map([clone_id], |r| {
                Ok(VoiceCloneAudioRow {
                    id: r.get(0)?,
                    clone_id: r.get(1)?,
                    file_path: r.get(2)?,
                    file_name: r.get(3)?,
                    transcript_lang: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_voice_clone_audio(&self, audio_id: i64) -> Result<Option<VoiceCloneAudioRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, clone_id, file_path, file_name, transcript_lang, created_at FROM voice_clone_audios WHERE id = ?1",
            [audio_id],
            |r| {
                Ok(VoiceCloneAudioRow {
                    id: r.get(0)?,
                    clone_id: r.get(1)?,
                    file_path: r.get(2)?,
                    file_name: r.get(3)?,
                    transcript_lang: r.get(4)?,
                    created_at: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_voice_clone(&self, id: i64, user_id: i64) -> Result<Option<VoiceCloneRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, user_id, tts_config_id, name, provider, status, voice_id, shared_to_all, transcript, error_message, created_at, updated_at
             FROM voice_clones WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
            map_voice_clone_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn create_voice_clone_task(
        &self,
        user_id: i64,
        voice_clone_id: i64,
        provider: &str,
    ) -> Result<VoiceCloneTaskRow> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();
        let meta_json = serde_json::json!({
            "task_id": task_id,
            "task_status": "queued",
            "queued_at": now,
        })
        .to_string();
        conn.execute(
            "INSERT INTO voice_clone_tasks (task_id, user_id, voice_clone_id, provider, status, attempts, last_error, meta_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'queued', 0, '', ?5, ?6, ?6)",
            params![task_id, user_id, voice_clone_id, provider, meta_json, now],
        )?;
        let id = conn.last_insert_rowid();
        conn.query_row(
            "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
             FROM voice_clone_tasks WHERE id = ?1",
            params![id],
            map_voice_clone_task_row,
        )
        .map_err(Into::into)
    }

    pub fn claim_voice_clone_task(&self, task_pk: i64) -> Result<Option<VoiceCloneTaskRow>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let existing: Option<VoiceCloneTaskRow> = tx
            .query_row(
                "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
                 FROM voice_clone_tasks WHERE id = ?1",
                params![task_pk],
                map_voice_clone_task_row,
            )
            .optional()?;
        let Some(task) = existing else {
            tx.commit()?;
            return Ok(None);
        };
        let status = task.status.trim().to_lowercase();
        if matches!(status.as_str(), "succeeded" | "failed") {
            tx.commit()?;
            return Ok(None);
        }
        if status != "queued" && status != "processing" {
            tx.commit()?;
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let updated = tx.execute(
            "UPDATE voice_clone_tasks SET status = 'processing', attempts = attempts + 1, last_error = '', started_at = ?1, finished_at = NULL, updated_at = ?1
             WHERE id = ?2 AND status IN ('queued', 'processing')",
            params![now, task_pk],
        )?;
        if updated == 0 {
            tx.commit()?;
            return Ok(None);
        }
        tx.execute(
            "UPDATE voice_clones SET status = 'processing', error_message = NULL, updated_at = ?1 WHERE id = ?2 AND user_id = ?3",
            params![now, task.voice_clone_id, task.user_id],
        )?;
        let claimed = tx.query_row(
            "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
             FROM voice_clone_tasks WHERE id = ?1",
            params![task_pk],
            map_voice_clone_task_row,
        )?;
        tx.commit()?;
        Ok(Some(claimed))
    }

    pub fn finish_voice_clone_task_success(&self, task_pk: i64, voice_id: &str) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        let task: VoiceCloneTaskRow = tx.query_row(
            "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
             FROM voice_clone_tasks WHERE id = ?1",
            params![task_pk],
            map_voice_clone_task_row,
        )?;
        let meta_json = serde_json::json!({
            "voice_id": voice_id,
            "finished_at": now,
        })
        .to_string();
        tx.execute(
            "UPDATE voice_clone_tasks SET status = 'succeeded', last_error = '', finished_at = ?1, meta_json = ?2, updated_at = ?1 WHERE id = ?3",
            params![now, meta_json, task_pk],
        )?;
        tx.execute(
            "UPDATE voice_clones SET status = 'active', voice_id = ?1, error_message = NULL, updated_at = ?2 WHERE id = ?3 AND user_id = ?4",
            params![voice_id, now, task.voice_clone_id, task.user_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish_voice_clone_task_failed(&self, task_pk: i64, error: &str) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        let task: VoiceCloneTaskRow = tx.query_row(
            "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
             FROM voice_clone_tasks WHERE id = ?1",
            params![task_pk],
            map_voice_clone_task_row,
        )?;
        let meta_json = serde_json::json!({
            "last_error": error,
            "finished_at": now,
        })
        .to_string();
        tx.execute(
            "UPDATE voice_clone_tasks SET status = 'failed', last_error = ?1, finished_at = ?2, meta_json = ?3, updated_at = ?2 WHERE id = ?4",
            params![error, now, meta_json, task_pk],
        )?;
        tx.execute(
            "UPDATE voice_clones SET status = 'failed', error_message = ?1, updated_at = ?2 WHERE id = ?3 AND user_id = ?4",
            params![error, now, task.voice_clone_id, task.user_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn requeue_failed_voice_clone_task(
        &self,
        voice_clone_id: i64,
        user_id: i64,
    ) -> Result<Option<i64>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let task: VoiceCloneTaskRow = tx.query_row(
            "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
             FROM voice_clone_tasks
             WHERE voice_clone_id = ?1 AND user_id = ?2
             ORDER BY created_at DESC, id DESC LIMIT 1",
            params![voice_clone_id, user_id],
            map_voice_clone_task_row,
        )?;
        if task.status.trim().to_lowercase() != "failed" {
            anyhow::bail!("当前任务状态为 {}，仅失败任务允许重新复刻", task.status);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let updated = tx.execute(
            "UPDATE voice_clone_tasks SET status = 'queued', last_error = '', started_at = NULL, finished_at = NULL, updated_at = ?1
             WHERE id = ?2 AND status = 'failed'",
            params![now, task.id],
        )?;
        if updated == 0 {
            anyhow::bail!("任务状态已变更，请刷新后重试");
        }
        tx.execute(
            "UPDATE voice_clones SET status = 'processing', error_message = NULL, updated_at = ?1 WHERE id = ?2 AND user_id = ?3",
            params![now, voice_clone_id, user_id],
        )?;
        tx.commit()?;
        Ok(Some(task.id))
    }

    pub fn get_voice_clone_task(&self, task_pk: i64) -> Result<Option<VoiceCloneTaskRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
             FROM voice_clone_tasks WHERE id = ?1",
            params![task_pk],
            map_voice_clone_task_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn latest_voice_clone_tasks_by_clone(
        &self,
        user_id: i64,
        clone_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, VoiceCloneTaskRow>> {
        let conn = self.conn.lock();
        let mut out = std::collections::HashMap::new();
        for clone_id in clone_ids {
            let task = conn
                .query_row(
                    "SELECT id, task_id, user_id, voice_clone_id, provider, status, attempts, last_error, started_at, finished_at, meta_json, created_at, updated_at
                     FROM voice_clone_tasks
                     WHERE user_id = ?1 AND voice_clone_id = ?2
                     ORDER BY created_at DESC, id DESC LIMIT 1",
                    params![user_id, clone_id],
                    map_voice_clone_task_row,
                )
                .optional()?;
            if let Some(task) = task {
                out.insert(*clone_id, task);
            }
        }
        Ok(out)
    }

    pub fn list_pending_voice_clone_task_ids(&self) -> Result<Vec<i64>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id FROM voice_clone_tasks WHERE status IN ('queued', 'processing') ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_knowledge_base(
        &self,
        id: i64,
        user_id: i64,
        name: &str,
        description: &str,
        provider: &str,
        status: &str,
        config_json: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE knowledge_bases SET name=?1, description=?2, provider=?3, status=?4, config_json=?5, updated_at=?6
             WHERE id=?7 AND user_id=?8",
            params![
                name,
                description,
                provider,
                status,
                config_json,
                chrono::Utc::now().to_rfc3339(),
                id,
                user_id
            ],
        )?;
        Ok(n > 0)
    }

    pub fn merge_kb_config_json(&self, id: i64, patch: &serde_json::Value) -> Result<()> {
        let current = self
            .get_knowledge_base(id)?
            .map(|kb| kb.config_json)
            .unwrap_or_else(|| "{}".to_string());
        let mut base: serde_json::Value =
            serde_json::from_str(&current).unwrap_or(serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = base {
            if let serde_json::Value::Object(patch_map) = patch {
                for (k, v) in patch_map {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        let merged = serde_json::to_string(&base).unwrap_or_else(|_| "{}".to_string());
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE knowledge_bases SET config_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![merged, chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn get_kb_document(&self, kb_id: i64, doc_id: i64) -> Result<Option<KbDocumentRow>> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, knowledge_base_id, title, content, source_type, status, external_doc_id, sync_error, created_at, updated_at
             FROM kb_documents WHERE id = ?1 AND knowledge_base_id = ?2",
            params![doc_id, kb_id],
            map_kb_document_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn update_kb_document_sync_state(
        &self,
        kb_id: i64,
        doc_id: i64,
        external_doc_id: Option<&str>,
        status: &str,
        sync_error: Option<&str>,
    ) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE kb_documents SET
                external_doc_id = CASE WHEN ?1 != '' THEN ?1 ELSE external_doc_id END,
                status = ?2,
                sync_error = ?3,
                updated_at = ?4
             WHERE id = ?5 AND knowledge_base_id = ?6",
            params![
                external_doc_id.unwrap_or(""),
                status,
                sync_error.unwrap_or(""),
                chrono::Utc::now().to_rfc3339(),
                doc_id,
                kb_id,
            ],
        )?;
        Ok(n > 0)
    }
    pub fn list_kb_documents(&self, kb_id: i64) -> Result<Vec<KbDocumentRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, knowledge_base_id, title, content, source_type, status, external_doc_id, sync_error, created_at, updated_at
             FROM kb_documents WHERE knowledge_base_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([kb_id], map_kb_document_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_kb_document(&self, kb_id: i64, title: &str, content: &str) -> Result<i64> {
        self.create_kb_document_with_meta(kb_id, title, content, "manual", "ready")
    }

    pub fn create_kb_document_with_meta(
        &self,
        kb_id: i64,
        title: &str,
        content: &str,
        source_type: &str,
        status: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kb_documents (knowledge_base_id, title, content, source_type, status, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![kb_id, title, content, source_type, status, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_kb_document(&self, kb_id: i64, doc_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM kb_documents WHERE id = ?1 AND knowledge_base_id = ?2",
            params![doc_id, kb_id],
        )?;
        Ok(n > 0)
    }

    pub fn update_kb_document(
        &self,
        kb_id: i64,
        doc_id: i64,
        title: &str,
        content: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().to_rfc3339();
        let n = conn.execute(
            "UPDATE kb_documents SET title = ?1, content = ?2, updated_at = ?3
             WHERE id = ?4 AND knowledge_base_id = ?5",
            params![title, content, now, doc_id, kb_id],
        )?;
        Ok(n > 0)
    }
}

/// 本地知识库关键词检索结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct KbSearchHit {
    pub knowledge_base_id: i64,
    pub document_id: i64,
    pub title: String,
    pub content: String,
    pub score: f64,
}

impl Database {
    pub fn search_kb_documents(
        &self,
        kb_ids: &[i64],
        query: &str,
        top_k: usize,
        threshold: f64,
    ) -> Result<Vec<KbSearchHit>> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || kb_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for kb_id in kb_ids {
            for doc in self.list_kb_documents(*kb_id)? {
                let score = score_kb_text(&doc.title, &doc.content, &q);
                if score >= threshold {
                    hits.push(KbSearchHit {
                        knowledge_base_id: *kb_id,
                        document_id: doc.id,
                        title: doc.title,
                        content: doc.content,
                        score,
                    });
                }
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(top_k);
        Ok(hits)
    }
}

fn score_kb_text(title: &str, content: &str, query: &str) -> f64 {
    let text = format!("{title} {content}").to_lowercase();
    if text.contains(query) {
        return 1.0;
    }
    let words: Vec<&str> = query.split_whitespace().filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return 0.0;
    }
    let matched = words.iter().filter(|w| text.contains(*w)).count();
    matched as f64 / words.len() as f64
}

fn map_agent_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRow> {
    Ok(AgentRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        name: r.get(2)?,
        system_prompt: r.get(3)?,
        llm_provider: r.get(4)?,
        llm_config: r.get(5)?,
        tts_provider: r.get(6)?,
        tts_config: r.get(7)?,
        asr_provider: r.get(8)?,
        asr_config: r.get(9)?,
        vad_provider: r.get(10)?,
        created_at: r.get(11)?,
        extra_json: r.get(12)?,
    })
}

pub fn normalize_device_mac(mac: &str) -> String {
    mac.trim().to_lowercase().replace('-', ":")
}

/// 0 或负数表示「不关联智能体」，写入数据库时应为 NULL
pub fn normalize_agent_id(agent_id: Option<i64>) -> Option<i64> {
    agent_id.filter(|id| *id > 0)
}

pub fn activation_bind_message(code: &str) -> String {
    format!("请在控制台输入验证码绑定设备，激活码: {code}")
}

fn migrate_backfill_chat_message_ownership(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE chat_messages
         SET user_id = (
           SELECT d.user_id FROM devices d
           WHERE lower(replace(d.device_id, '-', ':')) = lower(replace(chat_messages.device_id, '-', ':'))
             AND d.user_id IS NOT NULL
           LIMIT 1
         )
         WHERE user_id IS NULL
           AND device_id != ''
           AND EXISTS (
             SELECT 1 FROM devices d
             WHERE lower(replace(d.device_id, '-', ':')) = lower(replace(chat_messages.device_id, '-', ':'))
               AND d.user_id IS NOT NULL
           )",
        [],
    )?;
    conn.execute(
        "UPDATE chat_messages
         SET agent_id = (
           SELECT d.agent_id FROM devices d
           WHERE lower(replace(d.device_id, '-', ':')) = lower(replace(chat_messages.device_id, '-', ':'))
             AND d.agent_id IS NOT NULL
           LIMIT 1
         )
         WHERE agent_id IS NULL
           AND device_id != ''
           AND EXISTS (
             SELECT 1 FROM devices d
             WHERE lower(replace(d.device_id, '-', ':')) = lower(replace(chat_messages.device_id, '-', ':'))
               AND d.agent_id IS NOT NULL
           )",
        [],
    )?;
    Ok(())
}

fn migrate_devices_nullable_user_id(conn: &Connection) -> Result<()> {
    let notnull: i64 = conn
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('devices') WHERE name='user_id'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(1);
    if notnull == 0 {
        cleanup_system_pending_user(conn)?;
        return Ok(());
    }

    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         CREATE TABLE devices_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER,
            device_id TEXT NOT NULL UNIQUE,
            client_id TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL DEFAULT '',
            activated INTEGER NOT NULL DEFAULT 0,
            activation_code TEXT NOT NULL DEFAULT '',
            agent_id INTEGER,
            role_name TEXT NOT NULL DEFAULT 'default',
            online INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            role_id INTEGER,
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY(agent_id) REFERENCES agents(id) ON DELETE SET NULL
         );
         INSERT INTO devices_new (
            id, user_id, device_id, client_id, name, activated, activation_code,
            agent_id, role_name, online, created_at, role_id
         )
         SELECT
            id, user_id, device_id, client_id, name, activated, activation_code,
            agent_id, role_name, online, created_at, role_id
         FROM devices;
         DROP TABLE devices;
         ALTER TABLE devices_new RENAME TO devices;
         PRAGMA foreign_keys=ON;",
    )?;
    cleanup_system_pending_user(conn)?;
    Ok(())
}

fn cleanup_system_pending_user(conn: &Connection) -> Result<()> {
    if let Ok(pending_id) = conn.query_row(
        "SELECT id FROM users WHERE username = '__unbound_devices__'",
        [],
        |r| r.get::<_, i64>(0),
    ) {
        conn.execute(
            "UPDATE devices SET user_id = NULL WHERE user_id = ?1",
            [pending_id],
        )?;
        conn.execute("DELETE FROM users WHERE id = ?1", [pending_id])?;
    }
    Ok(())
}

fn map_device_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        device_id: r.get(2)?,
        client_id: r.get(3)?,
        name: r.get(4)?,
        activated: r.get::<_, i64>(5)? != 0,
        activation_code: r.get(6)?,
        agent_id: r.get(7)?,
        role_name: r.get(8)?,
        online: r.get::<_, i64>(9)? != 0,
        created_at: r.get(10)?,
        last_active_at: r.get(11)?,
    })
}

fn map_role_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RoleRow> {
    Ok(RoleRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        name: r.get(2)?,
        description: r.get(3)?,
        prompt: r.get(4)?,
        llm_config_id: r.get(5)?,
        tts_config_id: r.get(6)?,
        voice: r.get(7)?,
        role_type: r.get(8)?,
        status: r.get(9)?,
        sort_order: r.get(10)?,
        is_default: r.get::<_, i64>(11)? != 0,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
    })
}

fn map_chat_session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatSessionSummaryRow> {
    Ok(ChatSessionSummaryRow {
        session_id: r.get(0)?,
        device_id: r.get(1)?,
        agent_id: r.get(2)?,
        user_id: r.get(3)?,
        message_count: r.get(4)?,
        user_message_count: r.get(5)?,
        started_at: r.get(6)?,
        updated_at: r.get(7)?,
        preview: r.get(8)?,
        last_preview: r.get(9)?,
    })
}

fn map_chat_message_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessageRow> {
    Ok(ChatMessageRow {
        id: r.get(0)?,
        message_id: r.get(1)?,
        device_id: r.get(2)?,
        agent_id: r.get(3)?,
        user_id: r.get(4)?,
        session_id: r.get(5)?,
        role: r.get(6)?,
        content: r.get(7)?,
        tool_call_id: r.get(8)?,
        tool_calls_json: r.get(9)?,
        audio_path: r.get(10)?,
        audio_format: r.get(11)?,
        audio_size: r.get(12)?,
        audio_duration: r.get(13)?,
        metadata: r.get(14)?,
        created_at: r.get(15)?,
    })
}

fn map_speaker_group_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SpeakerGroupRow> {
    Ok(SpeakerGroupRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        agent_id: r.get(2)?,
        name: r.get(3)?,
        prompt: r.get(4)?,
        description: r.get(5)?,
        tts_config_id: r.get(6)?,
        voice: r.get(7)?,
        status: r.get(8)?,
        sample_count: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn map_voice_clone_task_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<VoiceCloneTaskRow> {
    Ok(VoiceCloneTaskRow {
        id: r.get(0)?,
        task_id: r.get(1)?,
        user_id: r.get(2)?,
        voice_clone_id: r.get(3)?,
        provider: r.get(4)?,
        status: r.get(5)?,
        attempts: r.get(6)?,
        last_error: r.get(7)?,
        started_at: r.get(8)?,
        finished_at: r.get(9)?,
        meta_json: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}

fn map_voice_clone_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<VoiceCloneRow> {
    Ok(VoiceCloneRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        tts_config_id: r.get(2)?,
        name: r.get(3)?,
        provider: r.get(4)?,
        status: r.get(5)?,
        voice_id: r.get(6)?,
        shared_to_all: r.get::<_, i64>(7)? != 0,
        transcript: r.get(8)?,
        error_message: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentRow {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub system_prompt: String,
    pub llm_provider: String,
    pub llm_config: String,
    pub tts_provider: String,
    pub tts_config: String,
    pub asr_provider: String,
    pub asr_config: String,
    pub vad_provider: String,
    pub created_at: String,
    pub extra_json: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceRow {
    pub id: i64,
    pub user_id: Option<i64>,
    pub device_id: String,
    pub client_id: String,
    pub name: String,
    pub activated: bool,
    pub activation_code: String,
    pub agent_id: Option<i64>,
    pub role_name: String,
    pub online: bool,
    pub created_at: String,
    pub last_active_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AgentInput {
    pub name: String,
    #[serde(default, alias = "custom_prompt")]
    pub system_prompt: String,
    #[serde(default, alias = "llm_config_id")]
    pub llm_provider: String,
    #[serde(default = "empty_json")]
    pub llm_config: String,
    #[serde(default, alias = "tts_config_id")]
    pub tts_provider: String,
    #[serde(default = "empty_json")]
    pub tts_config: String,
    #[serde(default)]
    pub asr_provider: String,
    #[serde(default = "empty_json")]
    pub asr_config: String,
    #[serde(default)]
    pub vad_provider: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default = "default_asr_speed")]
    pub asr_speed: String,
    #[serde(default = "default_memory_mode")]
    pub memory_mode: String,
    #[serde(default = "default_speaker_chat_mode")]
    pub speaker_chat_mode: String,
    #[serde(default)]
    pub mcp_service_names: String,
    #[serde(default, alias = "openclaw_config")]
    pub openclaw: Option<serde_json::Value>,
    #[serde(default)]
    pub knowledge_base_ids: Vec<i64>,
}

impl AgentInput {
    pub fn resolved_name(&self) -> String {
        let nickname = self.nickname.trim();
        if !nickname.is_empty() {
            nickname.to_string()
        } else {
            self.name.trim().to_string()
        }
    }

    pub fn extra_json(&self) -> String {
        serde_json::json!({
            "nickname": self.nickname,
            "voice": self.voice,
            "asr_speed": self.asr_speed,
            "memory_mode": self.memory_mode,
            "speaker_chat_mode": self.speaker_chat_mode,
            "mcp_service_names": self.mcp_service_names,
            "openclaw": self.openclaw.clone().unwrap_or(serde_json::json!({})),
            "knowledge_base_ids": self.knowledge_base_ids,
        })
        .to_string()
    }
}

fn default_asr_speed() -> String {
    "normal".into()
}

fn default_memory_mode() -> String {
    "short".into()
}

fn default_speaker_chat_mode() -> String {
    "off".into()
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeviceInput {
    #[serde(default, alias = "device_name")]
    pub device_id: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default, alias = "nick_name")]
    pub name: String,
    pub agent_id: Option<i64>,
    #[serde(default)]
    pub user_id: Option<i64>,
}

impl DeviceInput {
    pub fn resolved_device_id(&self) -> String {
        self.device_id.clone()
    }

    pub fn resolved_name(&self) -> String {
        self.name.clone()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigRow {
    pub id: i64,
    pub r#type: String,
    pub name: String,
    pub config_id: String,
    pub provider: String,
    pub json_data: String,
    pub enabled: bool,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConfigInput {
    #[serde(default)]
    pub r#type: String,
    pub name: String,
    pub config_id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default = "empty_json")]
    pub json_data: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserListRow {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub role: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RoleRow {
    pub id: i64,
    pub user_id: Option<i64>,
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub llm_config_id: Option<String>,
    pub tts_config_id: Option<String>,
    pub voice: Option<String>,
    pub role_type: String,
    pub status: String,
    pub sort_order: i64,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RoleInput {
    pub user_id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub llm_config_id: Option<String>,
    #[serde(default)]
    pub tts_config_id: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default = "default_role_type")]
    pub role_type: String,
    #[serde(default = "default_active_status")]
    pub status: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub is_default: bool,
}

fn default_role_type() -> String {
    "user".to_string()
}

fn default_active_status() -> String {
    "active".to_string()
}

#[derive(Debug, Clone)]
pub struct KnowledgeBaseDetail {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub provider: String,
    pub status: String,
    pub config_json: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeBaseRow {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub status: String,
    #[serde(default = "empty_json")]
    pub config_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeBaseListItem {
    #[serde(flatten)]
    pub base: KnowledgeBaseRow,
    pub doc_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatMessageRow {
    pub id: i64,
    pub message_id: String,
    pub device_id: String,
    pub agent_id: Option<i64>,
    pub user_id: Option<i64>,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_calls_json: Option<String>,
    pub audio_path: Option<String>,
    pub audio_format: Option<String>,
    pub audio_size: Option<i64>,
    pub audio_duration: Option<f64>,
    pub metadata: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatSessionSummaryRow {
    pub session_id: String,
    pub device_id: String,
    pub agent_id: Option<i64>,
    pub user_id: Option<i64>,
    pub message_count: i64,
    pub user_message_count: i64,
    pub preview: String,
    pub last_preview: String,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ChatSessionQuery {
    pub user_id: i64,
    pub agent_id: Option<i64>,
    pub device_id: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AdminChatSessionQuery {
    pub user_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub device_id: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ChatMessageQuery {
    pub user_id: i64,
    pub agent_id: Option<i64>,
    pub device_id: Option<String>,
    pub session_id: Option<String>,
    pub role: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AdminChatMessageQuery {
    pub user_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub device_id: Option<String>,
    pub session_id: Option<String>,
    pub role: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChatMessageInput {
    #[serde(default)]
    pub message_id: String,
    pub device_id: String,
    pub agent_id: Option<i64>,
    pub user_id: Option<i64>,
    #[serde(default)]
    pub session_id: String,
    pub role: String,
    #[serde(default)]
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_calls_json: Option<String>,
    #[serde(default)]
    pub metadata: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeakerGroupRow {
    pub id: i64,
    pub user_id: i64,
    pub agent_id: i64,
    pub name: String,
    pub prompt: String,
    pub description: String,
    pub tts_config_id: Option<String>,
    pub voice: Option<String>,
    pub status: String,
    pub sample_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SpeakerGroupInput {
    pub agent_id: i64,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub description: String,
    pub tts_config_id: Option<String>,
    pub voice: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeakerSampleRow {
    pub id: i64,
    pub group_id: i64,
    pub file_path: String,
    pub file_name: String,
    pub duration: Option<f64>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceCloneTaskRow {
    pub id: i64,
    pub task_id: String,
    pub user_id: i64,
    pub voice_clone_id: i64,
    pub provider: String,
    pub status: String,
    pub attempts: i64,
    pub last_error: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub meta_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceCloneRow {
    pub id: i64,
    pub user_id: i64,
    pub tts_config_id: String,
    pub name: String,
    pub provider: String,
    pub status: String,
    pub voice_id: Option<String>,
    pub shared_to_all: bool,
    pub transcript: String,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct VoiceCloneInput {
    pub tts_config_id: String,
    pub name: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub transcript: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceCloneQuotaRow {
    pub id: i64,
    pub user_id: i64,
    pub tts_config_id: String,
    pub max_count: i64,
    pub used_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceCloneAudioRow {
    pub id: i64,
    pub clone_id: i64,
    pub file_path: String,
    pub file_name: String,
    #[serde(default = "default_transcript_lang")]
    pub transcript_lang: String,
    pub created_at: String,
}

fn default_transcript_lang() -> String {
    "zh-CN".to_string()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct KbDocumentRow {
    pub id: i64,
    pub knowledge_base_id: i64,
    pub title: String,
    pub content: String,
    pub source_type: String,
    pub status: String,
    #[serde(default)]
    pub external_doc_id: String,
    #[serde(default)]
    pub sync_error: String,
    pub created_at: String,
    pub updated_at: String,
}

fn map_kb_document_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<KbDocumentRow> {
    Ok(KbDocumentRow {
        id: r.get(0)?,
        knowledge_base_id: r.get(1)?,
        title: r.get(2)?,
        content: r.get(3)?,
        source_type: r.get(4)?,
        status: r.get(5)?,
        external_doc_id: r.get(6)?,
        sync_error: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiTokenRow {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub token_prefix: String,
    pub expires_at: Option<String>,
    pub created_at: String,
}

fn purge_ota_probe_devices(conn: &Connection) -> Result<()> {
    let probe_id = xiaozhi_core::constants::ota_test::DEVICE_ID;
    conn.execute(
        "DELETE FROM activation_challenges WHERE LOWER(device_id) = LOWER(?1)",
        [probe_id],
    )?;
    conn.execute(
        "DELETE FROM devices WHERE LOWER(device_id) = LOWER(?1)",
        [probe_id],
    )?;
    Ok(())
}

fn filter_list_devices(rows: Vec<DeviceRow>) -> Vec<DeviceRow> {
    rows.into_iter()
        .filter(|d| {
            !xiaozhi_core::constants::simulator::is_hidden_list_device(&d.device_id)
        })
        .collect()
}

fn default_true() -> bool {
    true
}

fn empty_json() -> String {
    "{}".to_string()
}

