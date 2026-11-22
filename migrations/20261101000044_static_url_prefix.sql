-- Static URL prefix rename (2026-11-22): the web resources moved
-- to the Spring Boot layout (resources/templates +
-- resources/static) and the router now mounts the static tree at
-- /static instead of /assets. Stored image paths name the served
-- prefix, so every row still pointing at /assets/ is rewritten to
-- /static/ — the files keep their names and places inside the
-- tree; only the route they are served under changes. This also
-- rewrites the seed rows written by migration 2, so a fresh
-- install ends up consistent without editing that migration.
UPDATE products
   SET image_path = '/static/' || substr(image_path, 9)
 WHERE image_path LIKE '/assets/%';

UPDATE product_images
   SET image_path = '/static/' || substr(image_path, 9)
 WHERE image_path LIKE '/assets/%';
