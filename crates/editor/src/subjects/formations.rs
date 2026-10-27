//! Which creature spawns walk and fight together, and which answer another:
//! the state behind the creature inspector's Formation and Links sections.
//!
//! ## What is read
//!
//! `creature_groups`, `creature_linking` and `creature_linking_template`
//! whole, one query each, kept until an apply lands
//! (`EditSession::database_writes`) and indexed by leader, member, master
//! and slave. The three are small (5,781, 407 and 13 rows on the reference
//! database), and the viewport draws the selected spawn's formation and
//! links from them on every frame. A view lays the project's store over the
//! rows it draws; see [`crate::subjects::keyed`].
//!
//! ## What an edit is
//!
//! A formation is the rows naming one leader. A member's slot is the point
//! `dist` yards from the leader at `angle` from the leader's facing
//! ([`Placed::slot`]); while the leader moves, the member walks at its slot.
//! A spawn joins a formation where it stands: its `dist` and `angle` are read
//! from where it and the leader stand and which way the leader faces
//! (`vale_mangos::formation::Member::placing`), so its slot is its spawn
//! point.
//! A spawn is in one formation at most, since `member_guid` is the key, so
//! joining another moves it. Making a member the leader rewrites every row
//! of the formation under the new leader, each placed where it stands, as
//! one undo entry. A link is one row per slave, so a slave has one master.
//!
//! The selection is what an edit acts on: the primary spawn and the spawns
//! shift-clicked beside it. Add Selected puts the others in the primary's
//! formation, or links them to it as slaves.

use crate::server::fresh::FreshReads;
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::subjects::keyed::{claimed_keys, overlay, put_row, Shown};
use crate::tools::places::triggers::remove_row;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_mangos::formation::{self, Link, Member, TemplateLink};
use vale_mangos::row::{Edits, Key};

/// Where a spawn stands, for placing a member: its guid, its position in
/// the world's own axes, and the radians it faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placed {
    pub(crate) guid: u64,
    pub(crate) at: Vec3,
    pub(crate) facing: f32,
}

impl Placed {
    /// A member's slot with this spawn as its leader: the point `dist`
    /// yards from it at `angle` from its facing, at its height.
    pub(crate) fn slot(&self, row: &Member) -> Vec3 {
        let (dx, dy) = row.offset(self.facing);
        Vec3::new(self.at.x + dx, self.at.y + dy, self.at.z)
    }

    /// How far `member` stands from its slot with this spawn as its
    /// leader, in yards across the ground.
    pub(crate) fn slot_distance(&self, member: Vec3, row: &Member) -> f32 {
        (member.truncate() - self.slot(row).truncate()).length()
    }
}

/// How far a member may stand from its slot, in yards, and still stand in
/// it. A placed member is off by float rounding only.
pub(crate) const IN_SLOT: f32 = 0.1;

/// The three tables as the database has them, with the indexes a view
/// finds its rows by.
#[derive(Debug, Default)]
pub(crate) struct Held {
    members: Vec<Member>,
    links: Vec<Link>,
    templates: Vec<TemplateLink>,
    by_member: HashMap<u64, usize>,
    by_leader: HashMap<u64, Vec<usize>>,
    by_slave: HashMap<u64, usize>,
    by_master: HashMap<u64, Vec<usize>>,
}

impl Held {
    fn new(members: Vec<Member>, links: Vec<Link>, templates: Vec<TemplateLink>) -> Held {
        let mut held = Held { members, links, templates, ..Held::default() };
        for (at, member) in held.members.iter().enumerate() {
            held.by_member.insert(member.member, at);
            held.by_leader.entry(member.leader).or_default().push(at);
        }
        for (at, link) in held.links.iter().enumerate() {
            held.by_slave.insert(link.slave, at);
            held.by_master.entry(link.master).or_default().push(at);
        }
        held
    }
}

/// One formation: its leader and every row naming it, removals included.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Formation {
    pub(crate) leader: u64,
    /// In member order. A leader's row naming itself is among them.
    pub(crate) rows: Vec<Shown<Member>>,
}

impl Formation {
    /// The group's options: every kept member's flags ORed, as
    /// `CreatureGroup::AddMember` makes them. The leader's own row adds
    /// nothing, since `AddMember` returns before reading it.
    pub(crate) fn options(&self) -> u32 {
        self.rows
            .iter()
            .filter(|shown| shown.kept() && !shown.row.is_leader_row())
            .fold(0, |options, shown| options | shown.row.flags)
    }

    /// The kept members other than the leader.
    pub(crate) fn members(&self) -> impl Iterator<Item = &Shown<Member>> {
        self.rows.iter().filter(|shown| shown.kept() && !shown.row.is_leader_row())
    }
}

/// What the Formation and Links sections are holding.
#[derive(Resource, Default)]
pub(crate) struct Formations {
    read: FreshReads<(), Held>,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::subject::flags::on_the_command_line`, which waits on this.
    pub(crate) scripted_done: bool,
}

impl Formations {
    /// Why the last read answered nothing, when it answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.read.trouble()
    }

    /// The tables, once read.
    pub(crate) fn held(&self) -> Option<&Held> {
        self.read.value()
    }

    /// This spawn's own row, as the project leaves it.
    pub(crate) fn member_row(&self, guid: u64, edits: &Edits) -> Option<Shown<Member>> {
        let held = self.held()?;
        let row = held.by_member.get(&guid).map(|at| &held.members[*at]);
        overlay(edits, formation::GROUPS, &formation::member_key(guid), row, Member::assignments, Member::from_row)
    }

    /// The leader of the formation this spawn is in: its own row's leader, or
    /// itself when a kept row names it as leader. `None` for a spawn in no
    /// formation.
    pub(crate) fn leader_of(&self, guid: u64, edits: &Edits) -> Option<u64> {
        if let Some(own) = self.member_row(guid, edits).filter(Shown::kept) {
            return Some(own.row.leader);
        }
        self.rows_of(guid, edits).iter().any(Shown::kept).then_some(guid)
    }

    /// The formation this spawn is in, or `None`.
    pub(crate) fn formation_of(&self, guid: u64, edits: &Edits) -> Option<Formation> {
        let leader = self.leader_of(guid, edits)?;
        Some(Formation { leader, rows: self.rows_of(leader, edits) })
    }

    /// Every row naming `leader`, removals included, in member order.
    fn rows_of(&self, leader: u64, edits: &Edits) -> Vec<Shown<Member>> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut keys: Vec<Key> = held
            .by_leader
            .get(&leader)
            .into_iter()
            .flatten()
            .map(|at| held.members[*at].key())
            .collect();
        keys.extend(claimed_keys(edits, formation::GROUPS));
        keys.sort();
        keys.dedup();
        let mut out: Vec<Shown<Member>> = keys
            .iter()
            .filter_map(|key| {
                let guid = key.first()?;
                let row = held.by_member.get(&guid).map(|at| &held.members[*at]);
                overlay(edits, formation::GROUPS, key, row, Member::assignments, Member::from_row)
            })
            .filter(|shown| shown.row.leader == leader)
            .collect();
        out.sort_by_key(|shown| shown.row.member);
        out
    }

    /// This spawn's link to its master, as the project leaves it.
    pub(crate) fn link_of(&self, slave: u64, edits: &Edits) -> Option<Shown<Link>> {
        let held = self.held()?;
        let row = held.by_slave.get(&slave).map(|at| &held.links[*at]);
        overlay(edits, formation::LINKING, &formation::link_key(slave), row, Link::assignments, Link::from_row)
    }

    /// The links naming this spawn as master, removals included.
    pub(crate) fn slaves_of(&self, master: u64, edits: &Edits) -> Vec<Shown<Link>> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut keys: Vec<Key> = held
            .by_master
            .get(&master)
            .into_iter()
            .flatten()
            .map(|at| held.links[*at].key())
            .collect();
        keys.extend(claimed_keys(edits, formation::LINKING));
        keys.sort();
        keys.dedup();
        let mut out: Vec<Shown<Link>> = keys
            .iter()
            .filter_map(|key| self.link_of(key.first()?, edits))
            .filter(|shown| shown.row.master == master)
            .collect();
        out.sort_by_key(|shown| shown.row.slave);
        out
    }

    /// The template links naming this creature entry as slave or master.
    pub(crate) fn template_links_of(&self, entry: u32, edits: &Edits) -> Vec<Shown<TemplateLink>> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut keys: Vec<Key> = held
            .templates
            .iter()
            .filter(|link| link.entry == entry || link.master_entry == entry)
            .map(TemplateLink::key)
            .collect();
        keys.extend(claimed_keys(edits, formation::LINKING_TEMPLATE));
        keys.sort();
        keys.dedup();
        keys.iter()
            .filter_map(|key| {
                let row = held.templates.iter().find(|link| link.key() == *key);
                overlay(edits, formation::LINKING_TEMPLATE, key, row, TemplateLink::assignments, TemplateLink::from_row)
            })
            .filter(|shown| shown.row.entry == entry || shown.row.master_entry == entry)
            .collect()
    }

    // -----------------------------------------------------------------------
    // Edits
    // -----------------------------------------------------------------------

    /// Put `members` in `leader`'s formation, each where it stands, as one
    /// undo entry. A member already in a formation is moved to this one; the
    /// leader itself is skipped. Answers the status line.
    pub(crate) fn join(&self, session: &mut EditSession, leader: Placed, members: &[Placed], now: f64) -> String {
        let joining: Vec<&Placed> = members.iter().filter(|member| member.guid != leader.guid).collect();
        if joining.is_empty() {
            return "nothing to add: select the spawns to add beside the leader".to_string();
        }
        let subject = format!("{} {} join", formation::GROUPS, leader.guid);
        session.as_one("Join formation", &subject, |session| {
            for member in &joining {
                let (dist, angle) = Member::placing(
                    (leader.at.x, leader.at.y),
                    leader.facing,
                    (member.at.x, member.at.y),
                );
                let flags = self
                    .member_row(member.guid, &session.rows.edits)
                    .filter(|shown| shown.kept() && shown.row.leader == leader.guid)
                    .map_or(formation::DEFAULT_MEMBER_FLAGS, |shown| shown.row.flags);
                let row = Member { flags, ..Member::new(leader.guid, member.guid, dist, angle) };
                self.write_member(session, &row, now);
            }
        });
        format!("{} spawn(s) in the formation led by {}", joining.len(), leader.guid)
    }

    /// Write one whole member row, keeping or rewriting what is at its key.
    fn write_member(&self, session: &mut EditSession, row: &Member, now: f64) {
        let held = self.held().and_then(|held| held.by_member.get(&row.member).map(|at| &held.members[*at]));
        let label = format!("{} {}", formation::GROUPS, row.key().text());
        let gesture = Gesture { label: "Edit formation", subject: &label, now };
        let columns = held.map(Member::assignments);
        put_row(session, formation::GROUPS, &row.key(), columns.as_deref(), &row.assignments(), gesture);
    }

    /// Move a member's slot to where it stands now: its `dist` and `angle`
    /// from its leader's position and facing.
    pub(crate) fn place(&self, session: &mut EditSession, leader: Placed, member: Placed, now: f64) {
        let Some(own) = self.member_row(member.guid, &session.rows.edits) else { return };
        let (dist, angle) = Member::placing((leader.at.x, leader.at.y), leader.facing, (member.at.x, member.at.y));
        self.write_member(session, &Member { dist, angle, ..own.row }, now);
    }

    /// Take a spawn out of its formation.
    pub(crate) fn leave(&self, session: &mut EditSession, guid: u64, now: f64) {
        let held = self.held().is_some_and(|held| held.by_member.contains_key(&guid));
        let key = formation::member_key(guid);
        let label = format!("{} {}", formation::GROUPS, key.text());
        let gesture = Gesture { label: "Leave formation", subject: &label, now };
        remove_row(session, formation::GROUPS, &key, held, gesture);
    }

    /// Lead the formation with `leader` instead: every row of it names the
    /// new leader, each member placed where it stands, the old leader becomes
    /// a member, and the new leader's row names itself. One undo entry.
    /// `placed` answers where a spawn of the formation stands; a member it
    /// cannot place keeps its distance and angle.
    pub(crate) fn make_leader(
        &self,
        session: &mut EditSession,
        leader: Placed,
        placed: impl Fn(u64) -> Option<Placed>,
        now: f64,
    ) -> Result<String, String> {
        let Some(formation) = self.formation_of(leader.guid, &session.rows.edits) else {
            return Err(format!("spawn {} is in no formation", leader.guid));
        };
        if formation.leader == leader.guid {
            return Err(format!("spawn {} already leads the formation", leader.guid));
        }
        let old = formation.leader;
        let subject = format!("{} {old} lead", formation::GROUPS);
        session.as_one("Change formation leader", &subject, |session| {
            let mut guids: Vec<u64> = formation.members().map(|shown| shown.row.member).collect();
            guids.push(old);
            for guid in guids {
                let flags = formation
                    .rows
                    .iter()
                    .find(|shown| shown.row.member == guid)
                    .map_or(formation::DEFAULT_MEMBER_FLAGS, |shown| shown.row.flags);
                let row = match guid == leader.guid {
                    true => Member { flags, ..Member::new(guid, guid, 0.0, 0.0) },
                    false => {
                        let (dist, angle) = match placed(guid) {
                            Some(member) => Member::placing(
                                (leader.at.x, leader.at.y),
                                leader.facing,
                                (member.at.x, member.at.y),
                            ),
                            None => formation
                                .rows
                                .iter()
                                .find(|shown| shown.row.member == guid)
                                .map_or((0.0, 0.0), |shown| (shown.row.dist, shown.row.angle)),
                        };
                        Member { flags, ..Member::new(leader.guid, guid, dist, angle) }
                    }
                };
                self.write_member(session, &row, now);
            }
        });
        Ok(format!("spawn {} leads the formation; {old} is a member", leader.guid))
    }

    /// Set one column of a member row.
    pub(crate) fn set_member_column(&self, session: &mut EditSession, guid: u64, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.by_member.get(&guid).map(|at| &held.members[*at]));
        let key = formation::member_key(guid);
        let label = format!("{} {} {column}", formation::GROUPS, key.text());
        let gesture = Gesture { label: "Edit formation", subject: &label, now };
        let columns = held.map(Member::assignments);
        crate::subjects::services::write_column(session, formation::GROUPS, &key, columns.as_deref(), column, value, gesture);
    }

    /// Link `slaves` to `master`, each with the default flag or the flag its
    /// link to this master already has. One undo entry.
    pub(crate) fn link(&self, session: &mut EditSession, master: u64, slaves: &[u64], now: f64) -> String {
        let linking: Vec<u64> = slaves.iter().copied().filter(|slave| *slave != master).collect();
        if linking.is_empty() {
            return "nothing to link: select the slaves beside the master".to_string();
        }
        let subject = format!("{} {master} link", formation::LINKING);
        session.as_one("Link spawns", &subject, |session| {
            for slave in &linking {
                let flag = self
                    .link_of(*slave, &session.rows.edits)
                    .filter(|shown| shown.kept() && shown.row.master == master)
                    .map_or(formation::DEFAULT_LINK_FLAG, |shown| shown.row.flag);
                let row = Link { flag, ..Link::new(*slave, master) };
                let held = self.held().and_then(|held| held.by_slave.get(slave).map(|at| &held.links[*at]));
                let label = format!("{} {}", formation::LINKING, row.key().text());
                let gesture = Gesture { label: "Link spawns", subject: &label, now };
                let columns = held.map(Link::assignments);
                put_row(session, formation::LINKING, &row.key(), columns.as_deref(), &row.assignments(), gesture);
            }
        });
        format!("{} spawn(s) linked to {master}", linking.len())
    }

    /// Remove a spawn's link to its master.
    pub(crate) fn unlink(&self, session: &mut EditSession, slave: u64, now: f64) {
        let held = self.held().is_some_and(|held| held.by_slave.contains_key(&slave));
        let key = formation::link_key(slave);
        let label = format!("{} {}", formation::LINKING, key.text());
        let gesture = Gesture { label: "Unlink spawn", subject: &label, now };
        remove_row(session, formation::LINKING, &key, held, gesture);
    }

    /// Set one column of a link.
    pub(crate) fn set_link_column(&self, session: &mut EditSession, slave: u64, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.by_slave.get(&slave).map(|at| &held.links[*at]));
        let key = formation::link_key(slave);
        let label = format!("{} {} {column}", formation::LINKING, key.text());
        let gesture = Gesture { label: "Edit link", subject: &label, now };
        let columns = held.map(Link::assignments);
        crate::subjects::services::write_column(session, formation::LINKING, &key, columns.as_deref(), column, value, gesture);
    }

    /// Link every spawn of `entry` on `map` to a spawn of `master_entry`,
    /// with the default flag and a search range of 0. Refused when the entry
    /// already has a template link on the map, which is the key.
    pub(crate) fn add_template_link(
        &self,
        session: &mut EditSession,
        entry: u32,
        map: u32,
        master_entry: u32,
        now: f64,
    ) -> Result<String, String> {
        let key = formation::template_link_key(entry, map);
        let held = self.held().and_then(|held| held.templates.iter().find(|link| link.key() == key));
        let shown = overlay(&session.rows.edits, formation::LINKING_TEMPLATE, &key, held, TemplateLink::assignments, TemplateLink::from_row);
        if shown.as_ref().is_some_and(Shown::kept) {
            return Err(format!("entry {entry} already has a template link on map {map}"));
        }
        let row = TemplateLink { entry, map, master_entry, flag: formation::DEFAULT_LINK_FLAG, search_range: 0 };
        let label = format!("{} {}", formation::LINKING_TEMPLATE, key.text());
        let gesture = Gesture { label: "Add template link", subject: &label, now };
        let columns = held.map(TemplateLink::assignments);
        put_row(session, formation::LINKING_TEMPLATE, &key, columns.as_deref(), &row.assignments(), gesture);
        Ok(format!("every spawn of {entry} on map {map} is linked to a spawn of {master_entry}"))
    }

    /// Remove a template link.
    pub(crate) fn remove_template_link(&self, session: &mut EditSession, key: &Key, now: f64) {
        let held = self.held().is_some_and(|held| held.templates.iter().any(|link| link.key() == *key));
        let label = format!("{} {}", formation::LINKING_TEMPLATE, key.text());
        let gesture = Gesture { label: "Remove template link", subject: &label, now };
        remove_row(session, formation::LINKING_TEMPLATE, key, held, gesture);
    }

    /// Set one column of a template link.
    pub(crate) fn set_template_column(&self, session: &mut EditSession, key: &Key, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.templates.iter().find(|link| link.key() == *key));
        let label = format!("{} {} {column}", formation::LINKING_TEMPLATE, key.text());
        let gesture = Gesture { label: "Edit template link", subject: &label, now };
        let columns = held.map(TemplateLink::assignments);
        crate::subjects::services::write_column(session, formation::LINKING_TEMPLATE, key, columns.as_deref(), column, value, gesture);
    }
}

// ---------------------------------------------------------------------------
// The read
// ---------------------------------------------------------------------------

/// Read the three tables while the creature tool is open, and forget them
/// when an apply has moved the tables.
fn read_the_rows(
    mut formations: ResMut<Formations>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<crate::tools::Tool>,
) {
    let Some(session) = session else { return };
    let writes = session.database_writes;
    formations.read.renew(writes);
    match formations.read.poll() {
        Some(Ok(())) => {
            if let Some(held) = formations.held() {
                info!(
                    "formations: {} group row(s), {} link(s), {} template link(s)",
                    held.members.len(),
                    held.links.len(),
                    held.templates.len()
                );
            }
        }
        Some(Err(e)) => warn!("formations: {e}"),
        None => {}
    }
    if *tool != crate::tools::Tool::Creatures || formations.read.is_read(&()) || formations.read.is_busy() {
        return;
    }
    formations.read.want(writes, (), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let members = db.rows(&formation::groups_query())?.iter().filter_map(Member::from_row).collect();
            let links = db.rows(&formation::links_query())?.iter().filter_map(Link::from_row).collect();
            let templates = db
                .rows(&formation::template_links_query())?
                .iter()
                .filter_map(TemplateLink::from_row)
                .collect();
            Ok(Held::new(members, links, templates))
        }))
    });
}

/// `--formation-add <guid>` and `--link-add <guid>`: Add Selected and Link
/// Selected as Slaves on `--spawn` and one other spawn, once the tables are
/// read and `--spawn` is selected.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut formations: ResMut<Formations>,
    mut session: Option<ResMut<EditSession>>,
    creatures: Res<crate::tools::spawns::creatures::Creatures>,
    time: Res<Time>,
) {
    if formations.scripted_done {
        return;
    }
    let (join, link) = (args.subjects.formation_add, args.subjects.link_add);
    if (join.is_none() && link.is_none()) || formations.trouble().is_some() {
        formations.scripted_done = true;
        return;
    }
    let Some(primary) = args.spawns.spawn else {
        warn!("--formation-add and --link-add need --spawn");
        formations.scripted_done = true;
        return;
    };
    let Some(session) = session.as_mut() else { return };
    if formations.held().is_none() || creatures.selected != Some(primary) {
        return;
    }
    let edits = session.rows.edits.clone();
    let placed = |guid: u64| {
        creatures
            .edited(guid, Some(&edits))
            .map(|spawn| Placed { guid, at: spawn.at, facing: spawn.orientation })
    };
    let now = time.elapsed_secs_f64();
    if let Some(guid) = join {
        let leader = formations.leader_of(primary, &edits).unwrap_or(primary);
        match placed(leader).zip(placed(guid)) {
            Some((leader, member)) => info!("--formation-add {guid}: {}", formations.join(session, leader, &[member], now)),
            None => warn!("--formation-add {guid}: the spawn or its leader is not on this map"),
        }
    }
    if let Some(guid) = link {
        info!("--link-add {guid}: {}", formations.link(session, primary, &[guid], now));
    }
    formations.scripted_done = true;
}

pub(crate) struct FormationsToolPlugin;

impl Plugin for FormationsToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Formations>()
            .add_systems(Update, (read_the_rows, on_the_command_line).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(members: Vec<Member>, links: Vec<Link>) -> Formations {
        let mut formations = Formations::default();
        formations.read.insert((), Held::new(members, links, Vec::new()));
        formations
    }

    /// A member's formation is found through its own row, a leader's through
    /// the rows naming it, and the options are the members' flags ORed.
    #[test]
    fn a_spawn_finds_its_formation_from_either_end() {
        let formations = holding(
            vec![
                Member { flags: 1, ..Member::new(100, 100, 0.0, 0.0) },
                Member { flags: 2, ..Member::new(100, 101, 3.0, 1.0) },
                Member { flags: 8, ..Member::new(100, 102, 3.0, 2.0) },
            ],
            Vec::new(),
        );
        let edits = Edits::default();
        let from_member = formations.formation_of(102, &edits).unwrap();
        let from_leader = formations.formation_of(100, &edits).unwrap();
        assert_eq!(from_member, from_leader);
        assert_eq!(from_leader.rows.len(), 3);
        assert_eq!(from_leader.members().count(), 2);
        assert_eq!(from_leader.options(), 10);
        assert!(formations.formation_of(500, &edits).is_none());
    }

    /// A member moved to another leader by an edit is in the new formation
    /// and not the old one, and a created row is found through the store.
    #[test]
    fn an_edit_moves_a_member_between_formations() {
        let formations = holding(vec![Member::new(100, 101, 3.0, 1.0)], Vec::new());
        let mut edits = Edits::default();
        edits.set(formation::GROUPS, &formation::member_key(101), "leader_guid", Some("200".into()));
        let row = Member::new(200, 201, 2.0, 0.5);
        edits.set_row_line(
            formation::GROUPS,
            &row.key(),
            Some(&crate::subjects::services::creation(&row.assignments()).to_line()),
        );
        assert!(formations.formation_of(100, &edits).is_none());
        let moved = formations.formation_of(101, &edits).unwrap();
        assert_eq!(moved.leader, 200);
        assert_eq!(moved.members().map(|shown| shown.row.member).collect::<Vec<_>>(), vec![101, 201]);
    }

    /// A master's slaves come from the links naming it.
    #[test]
    fn a_master_finds_its_slaves() {
        let formations = holding(Vec::new(), vec![Link::new(11, 10), Link::new(12, 10), Link::new(13, 99)]);
        let edits = Edits::default();
        let slaves: Vec<u64> = formations.slaves_of(10, &edits).iter().map(|shown| shown.row.slave).collect();
        assert_eq!(slaves, vec![11, 12]);
        assert_eq!(formations.link_of(13, &edits).unwrap().row.master, 99);
    }

    /// A member placed where it stands has its slot there, whichever way the
    /// leader faces, and a slot turns with the leader.
    #[test]
    fn a_placed_member_stands_in_its_slot() {
        let leader = Placed { guid: 1, at: Vec3::new(100.0, 50.0, 7.0), facing: 2.0 };
        let stands = Vec3::new(97.0, 54.0, 9.0);
        let (dist, angle) = Member::placing((leader.at.x, leader.at.y), leader.facing, (stands.x, stands.y));
        let row = Member::new(1, 2, dist, angle);
        assert!(leader.slot_distance(stands, &row) < IN_SLOT);
        assert_eq!(leader.slot(&row).z, leader.at.z);
        let turned = Placed { facing: leader.facing + std::f32::consts::PI, ..leader };
        assert!((turned.slot_distance(stands, &row) - 2.0 * dist).abs() < 1e-3);
    }
}
