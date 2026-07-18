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
    /// Fenêtre de validité du support CTC (dates SMP brutes, v0.4.0).
    /// On stocke les dates, JAMAIS l'état : il se recalcule à la lecture
    /// (bascule automatique en « prêt » le jour de l'activation).
    pub ctc_activation: Option<String>,
    pub ctc_expiration: Option<String>,
}

/// État du dernier chargement de l'annuaire Peppol (table meta 1-ligne).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirStatus {
    pub loaded_at: i64,
    pub count: i64,
    pub source: String,
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
  note              TEXT,
  ctc_activation    TEXT,
  ctc_expiration    TEXT
);
CREATE TABLE IF NOT EXISTS peppol_directory_meta (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  loaded_at  INTEGER NOT NULL,
  count      INTEGER NOT NULL,
  source     TEXT NOT NULL
);
";

const UPSERT_SQL: &str = "INSERT INTO resolutions
 (participant, exists_in_peppol, pa_code, pa_name, pa_country,
  extended_ctc_fr, api_status, resolved_at, note, ctc_activation,
  ctc_expiration)
 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
 ON CONFLICT(participant) DO UPDATE SET
   exists_in_peppol=excluded.exists_in_peppol,
   pa_code=excluded.pa_code, pa_name=excluded.pa_name,
   pa_country=excluded.pa_country,
   extended_ctc_fr=excluded.extended_ctc_fr,
   api_status=excluded.api_status, resolved_at=excluded.resolved_at,
   note=excluded.note, ctc_activation=excluded.ctc_activation,
   ctc_expiration=excluded.ctc_expiration";

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
        // Migration : les bases d'avant une colonne sont complétées à
        // l'ouverture (note : v0.3.2 ; fenêtre CTC : v0.4.0).
        for col in ["note", "ctc_activation", "ctc_expiration"] {
            let present: bool = conn
                .prepare("SELECT 1 FROM pragma_table_info('resolutions') WHERE name=?1")
                .and_then(|mut s| s.exists([col]))
                .map_err(|e| e.to_string())?;
            if !present {
                conn.execute(&format!("ALTER TABLE resolutions ADD COLUMN {col} TEXT"), [])
                    .map_err(|e| e.to_string())?;
            }
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
            &r.ctc_activation,
            &r.ctc_expiration,
        )
    }

    pub fn get(&self, pid: &str) -> Result<Option<Resolution>, String> {
        self.conn
            .query_row(
                "SELECT participant, exists_in_peppol, pa_code, pa_name, pa_country,
                        extended_ctc_fr, api_status, resolved_at, note,
                        ctc_activation, ctc_expiration
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
                        extended_ctc_fr, api_status, resolved_at, note,
                        ctc_activation, ctc_expiration
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
            ctc_activation: row.get(9)?,
            ctc_expiration: row.get(10)?,
        })
    }

    /// Recrée entièrement `peppol_directory` (DROP+CREATE) et y insère les
    /// valeurs (INSERT OR IGNORE — la PK déduplique), puis met à jour la meta,
    /// le tout dans UNE transaction : un échec laisse l'ancien contenu intact
    /// et l'horodatage ne peut pas diverger du contenu. Renvoie le nombre de
    /// lignes distinctes réellement en table.
    pub fn replace_peppol_directory(
        &self,
        values: &[String],
        source: &str,
        loaded_at: i64,
    ) -> Result<usize, String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(
            "DROP TABLE IF EXISTS peppol_directory;
             CREATE TABLE peppol_directory (value TEXT PRIMARY KEY);",
        )
        .map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare_cached("INSERT OR IGNORE INTO peppol_directory (value) VALUES (?1)")
                .map_err(|e| e.to_string())?;
            for v in values {
                stmt.execute(params![v]).map_err(|e| e.to_string())?;
            }
        }
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM peppol_directory", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO peppol_directory_meta (id, loaded_at, count, source)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               loaded_at=excluded.loaded_at, count=excluded.count, source=excluded.source",
            params![loaded_at, count, source],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(count as usize)
    }

    /// État du dernier chargement de l'annuaire ; `None` si jamais chargé.
    pub fn peppol_directory_status(&self) -> Result<Option<DirStatus>, String> {
        self.conn
            .query_row(
                "SELECT loaded_at, count, source FROM peppol_directory_meta WHERE id = 1",
                [],
                |r| {
                    Ok(DirStatus {
                        loaded_at: r.get(0)?,
                        count: r.get(1)?,
                        source: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
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
            ctc_activation: None,
            ctc_expiration: None,
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
    fn fenetre_ctc_persistee_en_aller_retour() {
        // On stocke les DATES, jamais l'état : un adressage « activation
        // 01/09 » doit basculer seul en « prêt » le jour venu — l'état se
        // recalcule à chaque lecture à partir de ces colonnes.
        let s = Store::open_in_memory().unwrap();
        let mut r = res("a::fenetre", true, 1);
        r.ctc_activation = Some("2026-09-01T00:00:00Z".into());
        r.ctc_expiration = Some("2036-09-01".into());
        s.upsert(&r).unwrap();
        let lu = s.get("a::fenetre").unwrap().unwrap();
        assert_eq!(lu.ctc_activation.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert_eq!(lu.ctc_expiration.as_deref(), Some("2036-09-01"));
        // Re-résolution sans dates : l'upsert écrase (support sans borne).
        s.upsert(&res("a::fenetre", true, 2)).unwrap();
        let lu = s.get("a::fenetre").unwrap().unwrap();
        assert!(lu.ctc_activation.is_none());
        assert!(lu.ctc_expiration.is_none());
    }

    #[test]
    fn ouverture_migre_une_base_v03_sans_colonnes_dates() {
        // Base v0.3.x : colonne note présente, pas les colonnes dates.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v03.db");
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
                   resolved_at       INTEGER NOT NULL,
                   note              TEXT
                 );
                 INSERT INTO resolutions VALUES
                   ('a::v03', 1, NULL, NULL, NULL, 1, 'ok', 42,
                    'support CTC : activation 2026-09-01');",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let vieux = s.get("a::v03").unwrap().unwrap();
        assert!(vieux.ctc_activation.is_none());
        assert_eq!(
            vieux.note.as_deref(),
            Some("support CTC : activation 2026-09-01")
        );
        let mut r = res("a::neuf", true, 43);
        r.ctc_activation = Some("2026-09-01".into());
        s.upsert(&r).unwrap();
        assert_eq!(
            s.get("a::neuf").unwrap().unwrap().ctc_activation.as_deref(),
            Some("2026-09-01")
        );
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

    #[test]
    fn directory_charge_dedup_et_compte() {
        let s = Store::open_in_memory().unwrap();
        let vals = vec!["000122308".to_string(), "0559".to_string(), "000122308".to_string()];
        let n = s.replace_peppol_directory(&vals, "file", 1000).unwrap();
        assert_eq!(n, 2, "la PK déduplique le doublon");
        let st = s.peppol_directory_status().unwrap().unwrap();
        assert_eq!(st.count, 2);
        assert_eq!(st.loaded_at, 1000);
        assert_eq!(st.source, "file");
    }

    #[test]
    fn directory_est_recreee_a_chaque_chargement() {
        let s = Store::open_in_memory().unwrap();
        s.replace_peppol_directory(&["a".into(), "b".into(), "c".into()], "file", 1).unwrap();
        // Deuxième chargement : contenu entièrement remplacé, pas cumulé.
        let n = s.replace_peppol_directory(&["x".into()], "download", 2).unwrap();
        assert_eq!(n, 1);
        let st = s.peppol_directory_status().unwrap().unwrap();
        assert_eq!(st.count, 1);
        assert_eq!(st.source, "download");
        assert_eq!(st.loaded_at, 2);
    }

    #[test]
    fn directory_status_none_avant_tout_chargement() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.peppol_directory_status().unwrap().is_none());
    }

    #[test]
    fn ouverture_cree_la_table_meta_annuaire() {
        // Une base préexistante sans peppol_directory_meta doit rester
        // ouvrable et gagner la table (migration idempotente).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sans_meta.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE resolutions (
                   participant TEXT PRIMARY KEY, exists_in_peppol INTEGER,
                   pa_code TEXT, pa_name TEXT, pa_country TEXT,
                   extended_ctc_fr INTEGER, api_status TEXT NOT NULL,
                   resolved_at INTEGER NOT NULL );",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!(s.peppol_directory_status().unwrap().is_none());
        s.replace_peppol_directory(&["z".into()], "file", 7).unwrap();
        assert_eq!(s.peppol_directory_status().unwrap().unwrap().count, 1);
    }
}
