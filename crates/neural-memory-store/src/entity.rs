//! Entity storage and the entity retrieval branch.
//!
//! The branch answers a narrow question the other two answer badly: *which
//! records are about the thing this query names, under any of its names?*
//! Lexical already finds "Gemma" in text saying "Gemma". What it cannot do is
//! resolve "the 12B model" to the entity a record actually names, and that
//! alias resolution is the branch's entire justification.

use neural_memory_domain::{entity_identity, extract_mentions, EntityDictionary, EntityTerms};
use rusqlite::params;

use crate::{Store, StoreError};

#[derive(Clone, Debug)]
pub struct EntityHit {
    pub record_digest: String,
    /// Distinct entities from the query found in this record. More shared
    /// entities is a stronger signal than one entity mentioned repeatedly, so
    /// repeats within a record do not inflate it.
    pub shared_entities: usize,
    pub matched: Vec<String>,
}

impl Store {
    /// Register an entity. Idempotent; the identity is its seal.
    pub fn put_entity(&self, e: &EntityTerms) -> Result<String, StoreError> {
        let id = entity_identity(e);
        let mut aliases = e.aliases.clone();
        aliases.sort();
        aliases.dedup();
        self.conn.execute(
            "INSERT OR REPLACE INTO entities (id, canonical_name, entity_type, aliases)
             VALUES (?1,?2,?3,?4)",
            params![
                id,
                e.canonical_name,
                e.entity_type,
                serde_json::to_string(&aliases).expect("json")
            ],
        )?;
        Ok(id)
    }

    /// Every declared entity, as the dictionary the extractor runs against.
    ///
    /// Rebuilt from the store rather than cached: the dictionary's identity is
    /// sealed over its contents, so a stale copy would attribute mentions to a
    /// dictionary that no longer exists.
    pub fn entity_dictionary(&self) -> Result<(EntityDictionary, Vec<EntityTerms>), StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT canonical_name, entity_type, aliases FROM entities ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let aliases: String = r.get(2)?;
            Ok(EntityTerms {
                canonical_name: r.get(0)?,
                entity_type: r.get(1)?,
                aliases: serde_json::from_str(&aliases).unwrap_or_default(),
            })
        })?;
        let mut terms = Vec::new();
        for row in rows {
            terms.push(row?);
        }
        Ok((EntityDictionary::new(&terms), terms))
    }

    /// Extract and store mentions for one record.
    ///
    /// Mentions for this extractor are replaced wholesale rather than merged:
    /// re-running a *different* dictionary must not leave spans behind that the
    /// current one would not produce.
    pub fn index_mentions(
        &self,
        record_digest: &str,
        text: &str,
        dict: &EntityDictionary,
    ) -> Result<usize, StoreError> {
        let extractor = dict.extractor_identity();
        self.conn.execute(
            "DELETE FROM mentions WHERE record_digest = ?1 AND extractor_identity = ?2",
            params![record_digest, extractor],
        )?;
        let mentions = extract_mentions(dict, text);
        for m in &mentions {
            self.conn.execute(
                "INSERT OR REPLACE INTO mentions
                   (record_digest, entity_id, start_offset, end_offset, extractor_identity)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    record_digest,
                    m.entity_identity,
                    m.start as i64,
                    m.end as i64,
                    extractor
                ],
            )?;
        }
        Ok(mentions.len())
    }

    /// Records mentioning any entity the query names.
    ///
    /// The query runs through the same extractor as the corpus, so "the 12B
    /// model" and "Gemma 4 12B Q5_K_M" resolve to the same entity and the same
    /// records. Using a different matcher on either side would make the branch
    /// unexplainable.
    pub fn entity_search(
        &self,
        query: &str,
        dict: &EntityDictionary,
        limit: usize,
        include_retired: bool,
    ) -> Result<Vec<EntityHit>, StoreError> {
        if dict.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: Vec<String> = {
            let mut v: Vec<String> = extract_mentions(dict, query)
                .into_iter()
                .map(|m| m.entity_identity)
                .collect();
            v.sort();
            v.dedup();
            v
        };
        if wanted.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = wanted.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let retired = if include_retired {
            ""
        } else {
            " AND m.superseded_at IS NULL AND m.retracted_at IS NULL"
        };
        let sql = format!(
            "SELECT me.record_digest, count(DISTINCT me.entity_id) AS shared,
                    group_concat(DISTINCT me.entity_id)
             FROM mentions me
             JOIN memories m ON m.record_digest = me.record_digest
             WHERE me.entity_id IN ({placeholders}){retired}
             GROUP BY me.record_digest
             ORDER BY shared DESC, me.record_digest
             LIMIT ?{}",
            wanted.len() + 1
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut binds: Vec<&dyn rusqlite::ToSql> =
            wanted.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let lim = limit as i64;
        binds.push(&lim);

        let rows = stmt.query_map(binds.as_slice(), |r| {
            let matched: Option<String> = r.get(2)?;
            Ok(EntityHit {
                record_digest: r.get(0)?,
                shared_entities: r.get::<_, i64>(1)? as usize,
                matched: matched
                    .map(|s| s.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Re-extract mentions for every record. Returns `(records, mentions)`.
    pub fn reindex_mentions(&self) -> Result<(usize, usize), StoreError> {
        let (dict, _) = self.entity_dictionary()?;
        if dict.is_empty() {
            return Ok((0, 0));
        }
        let rows: Vec<(String, String)> = {
            let mut st = self
                .conn
                .prepare("SELECT record_digest, claim FROM memories ORDER BY recorded_seq")?;
            let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut v = Vec::new();
            for x in it {
                v.push(x?);
            }
            v
        };
        let mut total = 0;
        for (d, claim) in &rows {
            total += self.index_mentions(d, claim, &dict)?;
        }
        Ok((rows.len(), total))
    }
}
