-- Qualified-invoice registration number (Wave 2 B7): the
-- shop's \u9069\u683c\u8acb\u6c42\u66f8\u767a\u884c\u4e8b\u696d\u8005\u767b\u9332\u756a\u53f7 (T + 13 digits).
--
-- Empty = not registered (or not recorded): the packing
-- slip and the receipt print exactly as before. The value
-- itself is the owner's fact to enter (D-5); this column
-- only gives it a structured, validated home.
ALTER TABLE shop_settings
    ADD COLUMN invoice_registration_number TEXT NOT NULL DEFAULT '';
