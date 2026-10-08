use rusqlite::{Connection, Result};

use crate::statutory::AnnualReportDraft;

pub fn get(conn: &Connection, year: i32) -> Result<Option<AnnualReportDraft>> {
    let mut stmt = conn.prepare(
        "SELECT year, member_count, meeting_date, meeting_time_from, meeting_time_to,
                tb_activities, tb_continuous, tb_outlook,
                pk_decisions, pk_activities_next_year
         FROM annual_report_draft WHERE year = ?1",
    )?;
    let mut rows = stmt.query([year])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(AnnualReportDraft {
        year: row.get(0)?,
        member_count: row.get(1)?,
        meeting_date: row.get(2)?,
        meeting_time_from: row.get(3)?,
        meeting_time_to: row.get(4)?,
        tb_activities: row.get(5)?,
        tb_continuous: row.get(6)?,
        tb_outlook: row.get(7)?,
        pk_decisions: row.get(8)?,
        pk_activities_next_year: row.get(9)?,
    }))
}

pub fn upsert(conn: &Connection, draft: &AnnualReportDraft) -> Result<()> {
    conn.execute(
        "INSERT INTO annual_report_draft
             (year, member_count, meeting_date, meeting_time_from, meeting_time_to,
              tb_activities, tb_continuous, tb_outlook,
              pk_decisions, pk_activities_next_year, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
         ON CONFLICT(year) DO UPDATE SET
             member_count       = excluded.member_count,
             meeting_date       = excluded.meeting_date,
             meeting_time_from  = excluded.meeting_time_from,
             meeting_time_to    = excluded.meeting_time_to,
             tb_activities      = excluded.tb_activities,
             tb_continuous      = excluded.tb_continuous,
             tb_outlook         = excluded.tb_outlook,
             pk_decisions       = excluded.pk_decisions,
             pk_activities_next_year = excluded.pk_activities_next_year,
             updated_at         = datetime('now')",
        rusqlite::params![
            draft.year,
            draft.member_count,
            draft.meeting_date,
            draft.meeting_time_from,
            draft.meeting_time_to,
            draft.tb_activities,
            draft.tb_continuous,
            draft.tb_outlook,
            draft.pk_decisions,
            draft.pk_activities_next_year,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::mod_tests::open_full_db;

    #[test]
    fn upsert_and_get_round_trip() {
        let conn = open_full_db();
        let draft = AnnualReportDraft {
            year: 2025,
            member_count: 2,
            meeting_date: "14.02.2026".to_string(),
            meeting_time_from: "16:00".to_string(),
            meeting_time_to: "19:00".to_string(),
            tb_activities: "Event descriptions.".to_string(),
            tb_continuous: "Ongoing work.".to_string(),
            tb_outlook: "Plans for 2026.".to_string(),
            pk_decisions: "Board discharged.".to_string(),
            pk_activities_next_year: "Continue support.".to_string(),
        };
        upsert(&conn, &draft).unwrap();
        let loaded = get(&conn, 2025).unwrap().unwrap();
        assert_eq!(loaded.year, 2025);
        assert_eq!(loaded.member_count, 2);
        assert_eq!(loaded.meeting_date, "14.02.2026");
        assert_eq!(loaded.tb_activities, "Event descriptions.");
    }

    #[test]
    fn upsert_overwrites_on_second_call() {
        let conn = open_full_db();
        let mut draft = AnnualReportDraft {
            year: 2025,
            ..AnnualReportDraft::default_for_year(2025)
        };
        upsert(&conn, &draft).unwrap();
        draft.tb_outlook = "Updated outlook.".to_string();
        upsert(&conn, &draft).unwrap();
        let loaded = get(&conn, 2025).unwrap().unwrap();
        assert_eq!(loaded.tb_outlook, "Updated outlook.");
    }

    #[test]
    fn get_returns_none_for_missing_year() {
        let conn = open_full_db();
        assert!(get(&conn, 1999).unwrap().is_none());
    }
}
