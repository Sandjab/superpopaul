use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Resolution {
    pub participant: String,
    pub exists_in_peppol: Option<bool>,
    pub pa_code: Option<String>,
    pub pa_name: Option<String>,
    pub pa_country: Option<String>,
    pub extended_ctc_fr: Option<bool>,
    pub api_status: String,
    pub resolved_at: i64,
    /// Note diagnostique du résolveur (ex. « ServiceGroup HTTP 403 on … »)
    /// quand exists=1 sans PA ni verdict CTC : catalogue SMP illisible.
    pub note: Option<String>,
}

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS resolutions (
  participant       TEXT PRIMARY KEY,
  exists_in_peppol  INTEGER,
  pa_code           TEXT,
  pa_name           TEXT,
  pa_country        TEXT,
  extended_ctc_fr   INTEGER,
  api_status        TEXT NOT NULL,
  resolved_at       INTEGER NOT NULL,
  note              TEXT
);
";

const UPSERT_SQL: &str = "INSERT INTO resolutions
 (participant, exists_in_peppol, pa_code, pa_name, pa_country,
  extended_ctc_fr, api_status, resolved_at, note)
 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
 ON CONFLICT(participant) DO UPDATE SET
   exists_in_peppol=excluded.exists_in_peppol,
   pa_code=excluded.pa_code, pa_name=excluded.pa_name,
   pa_country=excluded.pa_country,
   extended_ctc_fr=excluded.extended_ctc_fr,
   api_status=excluded.api_status, resolved_at=excluded.resolved_at,
   note=excluded.note";

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        // WAL + synchronous=FULL forcerait un fsync à chaque commit ;
        // NORMAL est le pairing standard sûr avec le WAL.
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        // Migration : les bases d'avant la colonne note sont complétées
        // à l'ouverture.
        let a_note: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('resolutions') WHERE name='note'")
            .and_then(|mut s| s.exists([]))
            .map_err(|e| e.to_string())?;
        if !a_note {
            conn.execute("ALTER TABLE resolutions ADD COLUMN note TEXT", [])
                .map_err(|e| e.to_string())?;
        }
        Ok(Store { conn })
    }

    pub fn upsert(&self, r: &Resolution) -> Result<(), String> {
        self.conn
            .execute(UPSERT_SQL, Self::upsert_params(r))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Écrit un paquet de résolutions dans une seule transaction, avec un
    /// prepared statement réutilisé. Le resolver écrit ~50 résultats par
    /// paquet : un autocommit (+fsync) par ligne serait le goulot.
    pub fn upsert_batch(&self, items: &[Resolution]) -> Result<(), String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        {
            let mut stmt = tx.prepare_cached(UPSERT_SQL).map_err(|e| e.to_string())?;
            for r in items {
                stmt.execute(Self::upsert_params(r))
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }

    fn upsert_params(r: &Resolution) -> impl rusqlite::Params + '_ {
        (
            &r.participant,
            &r.exists_in_peppol,
            &r.pa_code,
            &r.pa_name,
            &r.pa_country,
            &r.extended_ctc_fr,
            &r.api_status,
            &r.resolved_at,
            &r.note,
        )
    }

    pub fn get(&self, pid: &str) -> Result<Option<Resolution>, String> {
        self.conn
            .query_row(
                "SELECT participant, exists_in_peppol, pa_code, pa_name, pa_country,
                        extended_ctc_fr, api_status, resolved_at, note
                 FROM resolutions WHERE participant = ?1",
                params![pid],
                Self::row_to_resolution,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Charge en mémoire les résolutions des PIDs demandés (calcul des modes,
    /// jointure de sortie). Par lots de 500 pour rester sous la limite de
    /// variables SQLite.
    pub fn load_map(&self, pids: &[String]) -> Result<HashMap<String, Resolution>, String> {
        let mut out = HashMap::with_capacity(pids.len());
        for chunk in pids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT participant, exists_in_peppol, pa_code, pa_name, pa_country,
                        extended_ctc_fr, api_status, resolved_at, note
                 FROM resolutions WHERE participant IN ({placeholders})"
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk), Self::row_to_resolution)
                .map_err(|e| e.to_string())?;
            for r in rows {
                let r = r.map_err(|e| e.to_string())?;
                out.insert(r.participant.clone(), r);
            }
        }
        Ok(out)
    }

    fn row_to_resolution(row: &rusqlite::Row<'_>) -> rusqlite::Result<Resolution> {
        Ok(Resolution {
            participant: row.get(0)?,
            exists_in_peppol: row.get(1)?,
            pa_code: row.get(2)?,
            pa_name: row.get(3)?,
            pa_country: row.get(4)?,
            extended_ctc_fr: row.get(5)?,
            api_status: row.get(6)?,
            resolved_at: row.get(7)?,
            note: row.get(8)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(pid: &str, ok: bool, at: i64) -> Resolution {
        Resolution {
            participant: pid.into(),
            exists_in_peppol: if ok { Some(true) } else { None },
            pa_code: if ok { Some("PA0042".into()) } else { None },
            pa_name: if ok { Some("ACME PA".into()) } else { None },
            pa_country: if ok { Some("FR".into()) } else { None },
            extended_ctc_fr: if ok { Some(true) } else { None },
            api_status: if ok { "ok".into() } else { "error:503".into() },
            resolved_at: at,
            note: None,
        }
    }

    #[test]
    fn upsert_puis_get() {
        let s = Store::open_in_memory().unwrap();
        s.upsert(&res("iso6523-actorid-upis::0009:1", true, 1000))
            .unwrap();
        let r = s.get("iso6523-actorid-upis::0009:1").unwrap().unwrap();
        assert_eq!(r.pa_code.as_deref(), Some("PA0042"));
        assert_eq!(r.api_status, "ok");
        // upsert écrase (re-résolution)
        s.upsert(&res("iso6523-actorid-upis::0009:1", true, 2000))
            .unwrap();
        assert_eq!(
            s.get("iso6523-actorid-upis::0009:1")
                .unwrap()
                .unwrap()
                .resolved_at,
            2000
        );
    }

    #[test]
    fn load_map_charge_uniquement_les_pids_demandes() {
        let s = Store::open_in_memory().unwrap();
        s.upsert(&res("a::1", true, 1)).unwrap();
        s.upsert(&res("a::2", false, 2)).unwrap();
        s.upsert(&res("a::3", true, 3)).unwrap();
        let m = s
            .load_map(&["a::1".into(), "a::2".into(), "a::inconnu".into()])
            .unwrap();
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("a::1"));
        assert!(!m.contains_key("a::3"));
    }

    #[test]
    fn get_absent_renvoie_none() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.get("a::zzz").unwrap().is_none());
    }

    #[test]
    fn upsert_batch_ecrit_tout_et_reste_relisible() {
        let s = Store::open_in_memory().unwrap();
        let items: Vec<Resolution> = (0..50).map(|i| res(&format!("b::{i}"), true, i)).collect();
        s.upsert_batch(&items).unwrap();
        let pids: Vec<String> = (0..50).map(|i| format!("b::{i}")).collect();
        let m = s.load_map(&pids).unwrap();
        assert_eq!(m.len(), 50);
        assert_eq!(m["b::49"].resolved_at, 49);
        // un batch vide ne plante pas
        s.upsert_batch(&[]).unwrap();
    }

    #[test]
    fn note_persistee_en_aller_retour() {
        // La note diagnostique (« ServiceGroup HTTP 403 on … ») doit survivre
        // en base : c'est elle qui distingue un ban WAF d'une panne SMP quand
        // exists=1 sans PA ni verdict CTC.
        let s = Store::open_in_memory().unwrap();
        let mut r = res("a::note", true, 1);
        r.note = Some("ServiceGroup HTTP 403 on https://smp.example".into());
        s.upsert(&r).unwrap();
        let lu = s.get("a::note").unwrap().unwrap();
        assert_eq!(
            lu.note.as_deref(),
            Some("ServiceGroup HTTP 403 on https://smp.example")
        );
        // Sans note : None en relecture (upsert écrase aussi la note).
        s.upsert(&res("a::note", true, 2)).unwrap();
        assert!(s.get("a::note").unwrap().unwrap().note.is_none());
    }

    #[test]
    fn ouverture_migre_une_base_sans_colonne_note() {
        // Les bases créées avant la colonne note doivent rester ouvrables,
        // relisibles (note=None) et accepter des upserts avec note.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ancienne.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE resolutions (
                   participant       TEXT PRIMARY KEY,
                   exists_in_peppol  INTEGER,
                   pa_code           TEXT,
                   pa_name           TEXT,
                   pa_country        TEXT,
                   extended_ctc_fr   INTEGER,
                   api_status        TEXT NOT NULL,
                   resolved_at       INTEGER NOT NULL
                 );
                 INSERT INTO resolutions VALUES ('a::vieux', 1, NULL, NULL, NULL, NULL, 'ok', 42);",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let vieux = s.get("a::vieux").unwrap().unwrap();
        assert!(vieux.note.is_none());
        let mut r = res("a::neuf", true, 43);
        r.note = Some("SMP catalogue indisponible".into());
        s.upsert(&r).unwrap();
        assert_eq!(
            s.get("a::neuf").unwrap().unwrap().note.as_deref(),
            Some("SMP catalogue indisponible")
        );
    }

    #[test]
    fn load_map_traverse_plusieurs_chunks() {
        let s = Store::open_in_memory().unwrap();
        let items: Vec<Resolution> = (0..600).map(|i| res(&format!("c::{i}"), true, i)).collect();
        s.upsert_batch(&items).unwrap();
        let pids: Vec<String> = (0..600).map(|i| format!("c::{i}")).collect();
        let m = s.load_map(&pids).unwrap();
        assert_eq!(m.len(), 600);
        assert!(m.contains_key("c::0"));
        assert!(m.contains_key("c::599"));
    }
}
