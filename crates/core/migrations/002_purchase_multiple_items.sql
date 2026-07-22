ALTER TABLE purchase ADD COLUMN multiple_items INTEGER NOT NULL DEFAULT 0;

-- Any purchase already linked to more than one inventory item must be
-- treated as multi-item so existing data remains valid after migration.
UPDATE purchase SET multiple_items = 1
 WHERE id IN (
   SELECT source_purchase_id
     FROM inventory_item
    WHERE source_purchase_id IS NOT NULL
    GROUP BY source_purchase_id
   HAVING COUNT(*) > 1
 );
