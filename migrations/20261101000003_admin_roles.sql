-- Admin accounts gain roles and permissions: an owner holds
-- every area and manages the other accounts; a staff account
-- holds only the areas listed in `permissions`. `is_active`
-- disables an account without deleting it (audit rows keep
-- pointing at the row).
ALTER TABLE public.admins
    ADD COLUMN role text NOT NULL DEFAULT 'staff',
    ADD COLUMN permissions text[] NOT NULL DEFAULT '{}',
    ADD COLUMN is_active boolean NOT NULL DEFAULT true;

ALTER TABLE public.admins
    ADD CONSTRAINT admins_role_check CHECK (role IN ('owner', 'staff'));

-- Every account that exists today predates roles and was
-- created by the operator's own bootstrap: it becomes an owner,
-- so the current sign-in keeps full access.
UPDATE public.admins SET role = 'owner';
