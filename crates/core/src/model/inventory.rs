use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Germany,
    Brazil,
}

impl Location {
    pub fn as_str(self) -> &'static str {
        match self {
            Location::Germany => "germany",
            Location::Brazil => "brazil",
        }
    }

    // Inherent method returning Option rather than impl std::str::FromStr,
    // consistent with the other domain enums in this codebase.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "germany" => Some(Location::Germany),
            "brazil" => Some(Location::Brazil),
            _ => None,
        }
    }

    /// Display-layer only (SPEC.md §5.1) — the stored value from `as_str()`
    /// stays an English identifier regardless of UI locale.
    pub fn label(self) -> String {
        match self {
            Location::Germany => rust_i18n::t!("status.location.germany"),
            Location::Brazil => rust_i18n::t!("status.location.brazil"),
        }
        .into_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Available,
    Reserved,
    Donated,
}

impl ItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemStatus::Available => "available",
            ItemStatus::Reserved => "reserved",
            ItemStatus::Donated => "donated",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "available" => Some(ItemStatus::Available),
            "reserved" => Some(ItemStatus::Reserved),
            "donated" => Some(ItemStatus::Donated),
            _ => None,
        }
    }

    /// The i18n key for "N items in this status" — separate from `label()`
    /// because a count needs lowercase wording and, in pt-BR, a plural.
    pub fn count_key(self, count: usize) -> &'static str {
        match (self, count == 1) {
            (ItemStatus::Available, true) => "inventory.group.count.available_one",
            (ItemStatus::Available, false) => "inventory.group.count.available_other",
            (ItemStatus::Reserved, true) => "inventory.group.count.reserved_one",
            (ItemStatus::Reserved, false) => "inventory.group.count.reserved_other",
            (ItemStatus::Donated, true) => "inventory.group.count.donated_one",
            (ItemStatus::Donated, false) => "inventory.group.count.donated_other",
        }
    }

    /// Display-layer only (SPEC.md §5.1) — the stored value from `as_str()`
    /// stays an English identifier regardless of UI locale.
    pub fn label(self) -> String {
        match self {
            ItemStatus::Available => rust_i18n::t!("status.item.available"),
            ItemStatus::Reserved => rust_i18n::t!("status.item.reserved"),
            ItemStatus::Donated => rust_i18n::t!("status.item.donated"),
        }
        .into_owned()
    }
}

/// Which items the consolidated Inventory list shows (SPEC.md §3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemFilter {
    #[default]
    All,
    NotDonated,
    DonatedOnly,
}

impl ItemFilter {
    pub const ALL: [ItemFilter; 3] = [
        ItemFilter::All,
        ItemFilter::NotDonated,
        ItemFilter::DonatedOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ItemFilter::All => "all",
            ItemFilter::NotDonated => "not_donated",
            ItemFilter::DonatedOnly => "donated",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "all" => Some(ItemFilter::All),
            "not_donated" => Some(ItemFilter::NotDonated),
            "donated" => Some(ItemFilter::DonatedOnly),
            _ => None,
        }
    }

    /// The i18n key for this filter's label, for callers to resolve in their
    /// own locale (`web` is explicit-locale, `desktop` ambient).
    pub fn label_key(self) -> &'static str {
        match self {
            ItemFilter::All => "inventory.filter.all",
            ItemFilter::NotDonated => "inventory.filter.not_donated",
            ItemFilter::DonatedOnly => "inventory.filter.donated",
        }
    }

    fn matches(self, status: ItemStatus) -> bool {
        match self {
            ItemFilter::All => true,
            ItemFilter::NotDonated => status != ItemStatus::Donated,
            ItemFilter::DonatedOnly => status == ItemStatus::Donated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    Donation,
    Purchase,
}

impl SourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceType::Donation => "donation",
            SourceType::Purchase => "purchase",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "donation" => Some(SourceType::Donation),
            "purchase" => Some(SourceType::Purchase),
            _ => None,
        }
    }
}

/// Inventory item joined with its category name and a human-readable source
/// description (donor name / "Anonymous" for donations, channel for purchases),
/// for list display.
#[derive(Clone)]
pub struct InventoryItemRow {
    pub id: i64,
    pub name: String,
    pub category_id: i64,
    pub category_name: String,
    pub source_type: SourceType,
    pub source_donation_id: Option<i64>,
    pub source_purchase_id: Option<i64>,
    pub source_desc: String,
    /// Date the item was acquired: `physical_donation.date_received` for
    /// donations, `purchase.date` for purchases. `None` only if the source
    /// row itself is missing (shouldn't happen given the FK, but the join
    /// is LEFT so it's not guaranteed at the type level).
    pub acquired_date: Option<String>,
    pub location: Location,
    pub status: ItemStatus,
    pub notes: Option<String>,
}

/// One category in the consolidated Inventory list (SPEC.md §3.8).
#[derive(Clone)]
pub struct CategoryGroup {
    pub category_id: i64,
    pub category_name: String,
    pub total: usize,
    pub names: Vec<NameGroup>,
}

/// All items in a category sharing one exact name, newest first.
#[derive(Clone)]
pub struct NameGroup {
    pub name: String,
    pub items: Vec<InventoryItemRow>,
}

impl NameGroup {
    /// How many of this group's items are in each status, omitting statuses
    /// with none, in a fixed display order.
    pub fn status_counts(&self) -> Vec<(ItemStatus, usize)> {
        [
            ItemStatus::Available,
            ItemStatus::Reserved,
            ItemStatus::Donated,
        ]
        .into_iter()
        .map(|status| {
            let count = self.items.iter().filter(|i| i.status == status).count();
            (status, count)
        })
        .filter(|(_, count)| *count > 0)
        .collect()
    }
}

/// Sort key for the consolidated list: case-insensitive, with accented Latin
/// letters filed under their base letter, so German and Portuguese names land
/// where a reader expects ("Äpfel" near "Apple", not after "Zebra").
fn sort_key(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .flat_map(|c| {
            let (first, second) = match c {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => ('a', None),
                'ç' => ('c', None),
                'è' | 'é' | 'ê' | 'ë' => ('e', None),
                'ì' | 'í' | 'î' | 'ï' => ('i', None),
                'ñ' => ('n', None),
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' => ('o', None),
                'ù' | 'ú' | 'û' | 'ü' => ('u', None),
                'ß' => ('s', Some('s')),
                other => (other, None),
            };
            std::iter::once(first).chain(second)
        })
        .collect()
}

/// Consolidates item rows for display: category, then exact name, then the
/// individual items. Rows stay one per physical unit underneath; this only
/// shapes what the list shows. Categories and names sort by `sort_key`, with
/// the exact spelling as a tie-break so the order is deterministic.
pub fn group_items(items: Vec<InventoryItemRow>, filter: ItemFilter) -> Vec<CategoryGroup> {
    type NameKey = (String, String);
    type CategoryKey = (String, i64);
    let mut tree: BTreeMap<CategoryKey, BTreeMap<NameKey, Vec<InventoryItemRow>>> = BTreeMap::new();
    for item in items.into_iter().filter(|i| filter.matches(i.status)) {
        tree.entry((sort_key(&item.category_name), item.category_id))
            .or_default()
            .entry((sort_key(&item.name), item.name.clone()))
            .or_default()
            .push(item);
    }
    tree.into_values()
        .map(|by_name| {
            let names: Vec<NameGroup> = by_name
                .into_iter()
                .map(|((_, name), mut items)| {
                    items.sort_by_key(|i| std::cmp::Reverse(i.id));
                    NameGroup { name, items }
                })
                .collect();
            // `tree` only holds entries created from an item, so every
            // category has at least one name with at least one item.
            let first = &names[0].items[0];
            CategoryGroup {
                category_id: first.category_id,
                category_name: first.category_name.clone(),
                total: names.iter().map(|n| n.items.len()).sum(),
                names,
            }
        })
        .collect()
}

#[derive(Clone)]
pub struct InventoryItemDraft {
    pub name: String,
    pub category_id: Option<i64>,
    pub source_type: SourceType,
    pub source_donation_id: Option<i64>,
    pub source_purchase_id: Option<i64>,
    pub location: Location,
    pub status: ItemStatus,
    pub notes: String,
}

impl Default for InventoryItemDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            category_id: None,
            source_type: SourceType::Donation,
            source_donation_id: None,
            source_purchase_id: None,
            location: Location::Germany,
            status: ItemStatus::Available,
            notes: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: i64,
        name: &str,
        category_id: i64,
        category: &str,
        status: ItemStatus,
    ) -> InventoryItemRow {
        InventoryItemRow {
            id,
            name: name.to_string(),
            category_id,
            category_name: category.to_string(),
            source_type: SourceType::Donation,
            source_donation_id: Some(1),
            source_purchase_id: None,
            source_desc: "Donor".to_string(),
            acquired_date: None,
            location: Location::Germany,
            status,
            notes: None,
        }
    }

    #[test]
    fn identical_names_collapse_into_one_group_newest_first() {
        let groups = group_items(
            vec![
                row(1, "Wheels 52mm", 1, "wheels", ItemStatus::Available),
                row(3, "Wheels 52mm", 1, "wheels", ItemStatus::Available),
                row(2, "Wheels 52mm", 1, "wheels", ItemStatus::Donated),
            ],
            ItemFilter::All,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].total, 3);
        assert_eq!(groups[0].names.len(), 1);
        let ids: Vec<i64> = groups[0].names[0].items.iter().map(|i| i.id).collect();
        assert_eq!(ids, [3, 2, 1]);
    }

    #[test]
    fn names_differing_only_by_case_stay_separate() {
        let groups = group_items(
            vec![
                row(1, "Wheels", 1, "wheels", ItemStatus::Available),
                row(2, "wheels", 1, "wheels", ItemStatus::Available),
            ],
            ItemFilter::All,
        );
        assert_eq!(groups[0].names.len(), 2);
        assert_eq!(groups[0].total, 2);
    }

    #[test]
    fn the_same_name_in_two_categories_appears_under_each() {
        let groups = group_items(
            vec![
                row(1, "Starter set", 1, "complete", ItemStatus::Available),
                row(2, "Starter set", 2, "other", ItemStatus::Available),
            ],
            ItemFilter::All,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].names[0].name, "Starter set");
        assert_eq!(groups[1].names[0].name, "Starter set");
    }

    #[test]
    fn categories_and_names_sort_case_insensitively() {
        let groups = group_items(
            vec![
                row(1, "zebra", 1, "Wheels", ItemStatus::Available),
                row(2, "Apple", 1, "Wheels", ItemStatus::Available),
                row(3, "Mango", 1, "Wheels", ItemStatus::Available),
                row(4, "Deck", 2, "deck", ItemStatus::Available),
            ],
            ItemFilter::All,
        );
        assert_eq!(groups[0].category_name, "deck");
        assert_eq!(groups[1].category_name, "Wheels");
        let names: Vec<&str> = groups[1].names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Apple", "Mango", "zebra"]);
    }

    #[test]
    fn accented_names_sort_under_their_base_letter() {
        let groups = group_items(
            vec![
                row(1, "Zebra", 1, "other", ItemStatus::Available),
                row(2, "Äpfel", 1, "other", ItemStatus::Available),
                row(3, "Patins", 1, "other", ItemStatus::Available),
                row(4, "Óculos", 1, "other", ItemStatus::Available),
                row(5, "Straße", 1, "other", ItemStatus::Available),
            ],
            ItemFilter::All,
        );
        let names: Vec<&str> = groups[0].names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Äpfel", "Óculos", "Patins", "Straße", "Zebra"]);
    }

    #[test]
    fn names_differing_only_by_accent_stay_separate() {
        let groups = group_items(
            vec![
                row(1, "Oculos", 1, "other", ItemStatus::Available),
                row(2, "Óculos", 1, "other", ItemStatus::Available),
            ],
            ItemFilter::All,
        );
        assert_eq!(groups[0].names.len(), 2);
    }

    #[test]
    fn status_counts_omit_statuses_with_no_items() {
        let groups = group_items(
            vec![
                row(1, "Deck", 1, "deck", ItemStatus::Donated),
                row(2, "Deck", 1, "deck", ItemStatus::Available),
                row(3, "Deck", 1, "deck", ItemStatus::Available),
            ],
            ItemFilter::All,
        );
        assert_eq!(
            groups[0].names[0].status_counts(),
            [(ItemStatus::Available, 2), (ItemStatus::Donated, 1)]
        );
    }

    fn mixed_rows() -> Vec<InventoryItemRow> {
        vec![
            row(1, "Deck", 1, "deck", ItemStatus::Available),
            row(2, "Deck", 1, "deck", ItemStatus::Reserved),
            row(3, "Deck", 1, "deck", ItemStatus::Donated),
            row(4, "Helmet", 2, "helmet", ItemStatus::Donated),
        ]
    }

    #[test]
    fn not_donated_filter_keeps_available_and_reserved_and_drops_empty_groups() {
        let groups = group_items(mixed_rows(), ItemFilter::NotDonated);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].category_name, "deck");
        assert_eq!(groups[0].total, 2);
    }

    #[test]
    fn donated_only_filter_counts_only_donated_items() {
        let groups = group_items(mixed_rows(), ItemFilter::DonatedOnly);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].total, 1);
        assert_eq!(groups[1].total, 1);
    }

    #[test]
    fn item_filter_round_trips_through_its_stored_form() {
        for filter in ItemFilter::ALL {
            assert_eq!(ItemFilter::from_str(filter.as_str()), Some(filter));
        }
        assert_eq!(ItemFilter::from_str("nonsense"), None);
    }
}
