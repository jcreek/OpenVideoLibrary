CREATE TABLE ovl_browse_order (
    file_id INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
    library_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    item_id INTEGER NOT NULL
);
CREATE INDEX ovl_browse_order_library_title ON ovl_browse_order(library_id,title,item_id,file_id);
INSERT INTO ovl_browse_order(file_id,library_id,title,item_id)
SELECT f.id,f.library_id,i.title,i.id FROM media_files f JOIN items i ON i.id=f.item_id;

CREATE TRIGGER ovl_browse_file_insert AFTER INSERT ON media_files BEGIN
    INSERT INTO ovl_browse_order(file_id,library_id,title,item_id)
    SELECT new.id,new.library_id,title,new.item_id FROM items WHERE id=new.item_id;
END;
CREATE TRIGGER ovl_browse_file_update AFTER UPDATE OF id,library_id,item_id ON media_files BEGIN
    DELETE FROM ovl_browse_order WHERE file_id=old.id;
    INSERT INTO ovl_browse_order(file_id,library_id,title,item_id)
    SELECT new.id,new.library_id,title,new.item_id FROM items WHERE id=new.item_id;
END;
CREATE TRIGGER ovl_browse_file_delete AFTER DELETE ON media_files BEGIN
    DELETE FROM ovl_browse_order WHERE file_id=old.id;
END;
CREATE TRIGGER ovl_browse_title_update AFTER UPDATE OF title ON items
WHEN old.title IS NOT new.title BEGIN
    UPDATE ovl_browse_order SET title=new.title
    WHERE file_id IN (SELECT id FROM media_files WHERE item_id=new.id);
END;
