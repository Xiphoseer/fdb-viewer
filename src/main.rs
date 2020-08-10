use assembly_data::fdb::{
    align::{Database, Field, Row, Table},
    core::ValueType,
};
use gio::prelude::*;
use gtk::{prelude::*, TreeView};
use memmap::Mmap;
use rusqlite::{types::ToSqlOutput, ToSql};
use std::{
    cell::RefCell,
    convert::TryFrom,
    fmt::Write,
    fs::File,
    io,
    ops::{Deref, Range},
    path::Path,
    rc::Rc,
    time::Instant,
};

struct DB {
    mmap: Mmap,
}

#[derive(Debug, Copy, Clone)]
struct Paging {
    num_pages: usize,
    current: usize,
}

struct TablePage {
    name: glib::GString,
    store: gtk::TreeStore,
}

fn try_load_file(path: &Path) -> io::Result<DB> {
    let _file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&_file)? };
    Ok(DB { mmap })
}

pub struct SqliteVal<'a>(Field<'a>);

impl<'a> ToSql for SqliteVal<'a> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        use rusqlite::types::Value;
        let r = match self.0 {
            Field::Nothing => Value::Null,
            Field::Integer(i) => Value::Integer(i.into()),
            Field::Float(f) => Value::Real(f.into()),
            Field::Text(s) => Value::Text(s.decode().into_owned()),
            Field::Boolean(b) => Value::Integer(if b { 1 } else { 0 }),
            Field::BigInt(i) => Value::Integer(i),
            Field::VarChar(b) => Value::Text(b.decode().into_owned()),
        };
        Ok(ToSqlOutput::Owned(r))
    }
}

struct Iter<'a> {
    inner: Box<dyn Iterator<Item = Field<'a>> + 'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = SqliteVal<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(SqliteVal)
    }
}

struct SqliteRow<'a>(Row<'a>);

impl<'a> IntoIterator for SqliteRow<'a> {
    type IntoIter = Iter<'a>;
    type Item = SqliteVal<'a>;
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            inner: Box::new(self.0.field_iter()),
        }
    }
}

fn try_export_db(path: &Path, db: Database) -> rusqlite::Result<()> {
    let start = Instant::now();
    let conn = rusqlite::Connection::open(path)?;

    conn.execute("BEGIN", rusqlite::params![])?;

    let tables = db.tables();
    for table in tables.iter() {
        let mut create_query = format!("CREATE TABLE IF NOT EXISTS \"{}\"\n(\n", table.name());
        let mut insert_query = format!("INSERT INTO \"{}\" (", table.name());
        let mut first = true;
        for col in table.column_iter() {
            if first {
                first = false;
            } else {
                writeln!(create_query, ",").unwrap();
                write!(insert_query, ", ").unwrap();
            }
            let typ = match col.value_type() {
                ValueType::Nothing => "NULL",
                ValueType::Integer => "INTEGER",
                ValueType::Float => "REAL",
                ValueType::Text => "TEXT",
                ValueType::Boolean => "INTEGER",
                ValueType::BigInt => "INTEGER",
                ValueType::VarChar => "BLOB",
                ValueType::Unknown(_) => panic!(),
            };
            write!(create_query, "    [{}] {}", col.name(), typ).unwrap();
            write!(insert_query, "[{}]", col.name()).unwrap();
        }
        create_query.push_str(");");
        insert_query.push_str(") VALUES (?1");
        for i in 2..=table.column_count() {
            write!(insert_query, ", ?{}", i).unwrap();
        }
        insert_query.push_str(");");
        println!("{}", insert_query);
        conn.execute(&create_query, rusqlite::params![])?;

        let mut stmt = conn.prepare(&insert_query)?;
        for row in table.row_iter() {
            stmt.execute(SqliteRow(row))?;
        }
    }

    conn.execute("COMMIT", rusqlite::params![])?;

    let duration = start.elapsed();
    println!(
        "Export finished in {}.{}s",
        duration.as_secs(),
        duration.as_millis() % 1000
    );
    Ok(())
}

pub enum RefField {
    Integer(i32),
    Float(f32),
    Text(String),
    Boolean(bool),
    BigInt(i64),
    VarChar(String),
}

impl RefField {
    fn from(field: Field) -> Option<Self> {
        match field {
            Field::Nothing => None,
            Field::Integer(iv) => Some(RefField::Integer(iv)),
            Field::Float(fv) => Some(RefField::Float(fv)),
            Field::Text(tv) => Some(RefField::Text(tv.decode().into_owned())),
            Field::Boolean(bv) => Some(RefField::Boolean(bv)),
            Field::BigInt(iv) => Some(RefField::BigInt(iv)),
            Field::VarChar(vv) => Some(RefField::VarChar(vv.decode().into_owned())),
        }
    }
}

fn display_table(
    table_content_store: &gtk::TreeStore,
    col_count: usize,
    table: Table,
    range: Range<usize>,
) -> usize {
    let mut buffer: Vec<RefField> = Vec::with_capacity(col_count);
    let mut gtval: Vec<&'static dyn glib::ToValue> = Vec::with_capacity(col_count);
    let mut gtidx = Vec::with_capacity(col_count);

    table_content_store.clear();

    let mut count: usize = 0;

    for row in table.row_iter() {
        if !range.contains(&count) {
            count += 1;
            continue;
        }

        buffer.clear();
        gtval.clear();
        gtidx.clear();

        for (i, field) in row.field_iter().enumerate() {
            if let Some(r) = RefField::from(field) {
                buffer.push(r);
                let cidex_u32 = u32::try_from(i).unwrap();
                gtidx.push(cidex_u32);
            }
        }

        for f in &buffer {
            match f {
                RefField::Integer(int_val) => {
                    let v: &'static i32 = unsafe { std::mem::transmute(int_val) };
                    gtval.push(v);
                }
                RefField::Float(float_val) => {
                    let v: &'static f32 = unsafe { std::mem::transmute(float_val) };
                    gtval.push(v);
                }
                RefField::Text(str_val) => {
                    let v: &'static String = unsafe { std::mem::transmute(str_val) };
                    gtval.push(v);
                }
                RefField::Boolean(bool_val) => {
                    let v: &'static bool = unsafe { std::mem::transmute(bool_val) };
                    gtval.push(v);
                }
                RefField::BigInt(int_val) => {
                    let v: &'static i64 = unsafe { std::mem::transmute(int_val) };
                    gtval.push(v);
                }
                RefField::VarChar(str_val) => {
                    let v: &'static String = unsafe { std::mem::transmute(str_val) };
                    gtval.push(v);
                }
            }
        }
        table_content_store.insert_with_values(None, None, &gtidx[..], &gtval[..]);
        count += 1;
    }
    count
}

fn main() {
    if gtk::init().is_err() {
        println!("Failed to initialize GTK.");
        return;
    }

    let database_memmap: Rc<RefCell<Option<DB>>> = Rc::new(RefCell::new(Option::None));

    let glade_src = include_str!("fdb-viewer.glade");
    let builder = gtk::Builder::from_string(glade_src);

    let additional_css = include_str!("fdb-viewer.css");
    let css_provider = gtk::CssProvider::new();
    css_provider
        .load_from_data(additional_css.as_bytes())
        .unwrap();

    let window: gtk::Window = builder.get_object("window").unwrap();
    let header: gtk::HeaderBar = builder.get_object("header").unwrap();
    let pane_container: gtk::Paned = builder.get_object("paned1").unwrap();

    let screen = gdk::Screen::get_default().unwrap();
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &css_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    let button_open: gtk::Button = builder.get_object("button-open").unwrap();
    let button_export: gtk::Button = builder.get_object("button-export").unwrap();
    let button_search: gtk::ToggleButton = builder.get_object("button-search").unwrap();
    let button_next: gtk::Button = builder.get_object("button-next").unwrap();
    let button_previous: gtk::Button = builder.get_object("button-previous").unwrap();
    let button_box_paging: gtk::ButtonBox = builder.get_object("button-box-paging").unwrap();
    let label_page: gtk::Label = builder.get_object("label-page").unwrap();

    // TODO: maybe later
    button_box_paging.set_visible(false);

    /*let hsize_group = gtk::SizeGroupBuilder::new()
    .mode(gtk::SizeGroupMode::Horizontal)
    .build();*/

    let left_box = gtk::BoxBuilder::new()
        .orientation(gtk::Orientation::Vertical)
        .build();

    let entry = gtk::SearchEntryBuilder::new()
        .placeholder_text("Search table...")
        .build();

    let searchbar = gtk::SearchBar::new();
    searchbar.add(&entry);
    searchbar.set_hexpand(false);

    let listbox = gtk::ListBox::new();
    listbox.get_style_context().add_class("fdb-table-list");
    listbox.set_size_request(250, 250);

    let add_table_row = {
        let listbox = listbox.clone();
        move |table: Table| {
            let name = table.name();
            let n = name.as_ref();
            let lbl = gtk::LabelBuilder::new().label(n).xalign(0.0).build();
            let row = gtk::ListBoxRow::new();
            row.get_style_context().add_class("fdb-table");
            row.add(&lbl);
            listbox.add(&row);
        }
    };

    /*listbox.set_header_func(Some(Box::new(|row: &gtk::ListBoxRow, before| {
        if before.is_some() && row.get_header().is_none() {
            let sep = gtk::SeparatorBuilder::new()
                .orientation(gtk::Orientation::Horizontal)
                .build();
            row.set_header(Some(&sep));
        }
    })));*/

    button_search.connect_toggled({
        let searchbar = searchbar.clone();
        let entry = entry.clone();
        let listbox = listbox.clone();

        move |btn| {
            if btn.get_active() {
                searchbar.set_search_mode(true);
                entry.grab_focus();
                listbox.set_filter_func({
                    let entry = entry.clone();
                    Some(Box::new(move |row: &gtk::ListBoxRow| {
                        let search = entry.get_text();
                        if search.is_empty() {
                            return true;
                        }

                        let label: gtk::Label = row.get_child().unwrap().downcast().unwrap();
                        let name = label.get_text();
                        name.contains(search.as_str())
                    }))
                });
            } else {
                listbox.set_filter_func(None);
                searchbar.set_search_mode(false);
                entry.set_text("");
            }
        }
    });

    entry.connect_search_changed({
        let listbox = listbox.clone();
        move |_entry| {
            listbox.invalidate_filter();
        }
    });

    let scroll = gtk::ScrolledWindowBuilder::new()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scroll.add(&listbox);

    left_box.pack_start(&searchbar, false, false, 0);
    left_box.pack_start(&scroll, true, true, 0);

    let table_content_view = gtk::TreeViewBuilder::new().build();
    let scroll2 = gtk::ScrolledWindowBuilder::new()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Always)
        .build();

    scroll2.add(&table_content_view);

    pane_container.add1(&left_box);
    pane_container.set_child_shrink(&left_box, false);
    pane_container.add2(&scroll2);

    fn append_text_column(tree: &TreeView, name: &str, col_index: usize) {
        let column = gtk::TreeViewColumn::new();
        let cell = gtk::CellRendererText::new();

        column.pack_start(&cell, true);
        column.set_title(name);
        let cidx_i32 = i32::try_from(col_index).unwrap();
        column.add_attribute(&cell, "text", cidx_i32);
        tree.append_column(&column);
    }

    let paging = Rc::new(RefCell::new(None));
    let page: Rc<RefCell<Option<TablePage>>> = Rc::new(RefCell::new(None));

    let set_paging = {
        let paging = paging.clone();
        //let button_box_paging = button_box_paging.clone();
        let button_previous = button_previous.clone();
        let button_next = button_next.clone();
        //let label_page = label_page.clone();
        move |new: Option<Paging>| {
            *paging.borrow_mut() = new;
            if let Some(p) = new {
                button_box_paging.set_visible(true);
                label_page.set_text(&format!("{}/{}", p.current + 1, p.num_pages));
                button_next.set_sensitive(p.current + 1 < p.num_pages);
                button_previous.set_sensitive(p.current > 0);
            } else {
                button_box_paging.set_visible(false);
            }
        }
    };

    button_previous.connect_clicked({
        let page = page.clone();
        let database_memmap = database_memmap.clone();
        let paging = paging.clone();
        let set_paging = set_paging.clone();
        move |_button_next| {
            if let Some(page) = page.borrow().deref() {
                let opt = *paging.borrow();
                if let Some(paging) = opt {
                    let b = database_memmap.borrow();
                    let mmap = &b.as_ref().unwrap().mmap[..];
                    let db: Database = Database::new(mmap);

                    let tables = db.tables();
                    let table = tables.by_name(page.name.as_str()).unwrap();

                    let current = paging.current - 1;
                    let new_min = current * 1024;
                    let new_max = new_min + 1024;
                    display_table(&page.store, table.column_count(), table, new_min..new_max);

                    set_paging(Some(Paging {
                        current,
                        num_pages: paging.num_pages,
                    }))
                }
            }
        }
    });

    button_next.connect_clicked({
        let page = page.clone();
        let database_memmap = database_memmap.clone();
        //let paging = paging.clone();
        let set_paging = set_paging.clone();
        move |_button_next| {
            if let Some(page) = page.borrow().deref() {
                let opt = *paging.borrow();
                if let Some(paging) = opt {
                    let b = database_memmap.borrow();
                    let mmap = &b.as_ref().unwrap().mmap[..];
                    let db: Database = Database::new(mmap);

                    let tables = db.tables();
                    let table = tables.by_name(page.name.as_str()).unwrap();

                    let current = paging.current + 1;
                    let new_min = current * 1024;
                    let new_max = new_min + 1024;
                    display_table(&page.store, table.column_count(), table, new_min..new_max);

                    set_paging(Some(Paging {
                        current,
                        num_pages: paging.num_pages,
                    }))
                }
            }
        }
    });

    listbox.connect_row_selected({
        //let table_content_view = table_content_view.clone();
        let database_memmap = database_memmap.clone();
        //let page = page.clone();
        //let set_paging = set_paging.clone();
        move |_list, obj| {
            if let Some(row) = obj {
                table_content_view.set_model::<gtk::TreeStore>(None);

                for col in table_content_view.get_columns() {
                    table_content_view.remove_column(&col);
                }

                let label: gtk::Label = row.get_child().unwrap().downcast().unwrap();
                let name = label.get_text();

                let b = database_memmap.borrow();
                let mmap = &b.as_ref().unwrap().mmap[..];
                let db: Database = Database::new(mmap);

                let tables = db.tables();
                let table = tables.by_name(name.as_str()).unwrap();

                let col_count = table.column_count();
                let mut gtcol = Vec::with_capacity(col_count);

                for (col_index, tcol) in table.column_iter().enumerate() {
                    let typ = match tcol.value_type() {
                        ValueType::Nothing => String::static_type(),
                        ValueType::Integer => i32::static_type(),
                        ValueType::Float => f32::static_type(),
                        ValueType::Text => String::static_type(),
                        ValueType::Boolean => bool::static_type(),
                        ValueType::BigInt => i64::static_type(),
                        ValueType::VarChar => String::static_type(),
                        ValueType::Unknown(k) => panic!("Column type unknown {}", k),
                    };
                    gtcol.push(typ);
                    append_text_column(&table_content_view, tcol.name().as_ref(), col_index);
                }

                let table_content_store = gtk::TreeStore::new(&gtcol[..]);
                let max = display_table(&table_content_store, col_count, table, 0..1024);
                let num_pages = (max / 1024) + 1;

                set_paging(Some(Paging {
                    num_pages,
                    current: 0,
                }));

                table_content_view.set_model(Some(&table_content_store));

                *page.borrow_mut() = Some(TablePage {
                    name,
                    store: table_content_store,
                });
            } else {
                println!("Unselect Row")
            }
        }
    });

    let load = {
        //let listbox = listbox.clone();
        let database_memmap = database_memmap.clone();
        let add_table_row = add_table_row.clone();
        let button_export = button_export.clone();
        move |db: DB| {
            listbox.forall({
                let listbox = listbox.clone();
                move |child| {
                    listbox.remove(child);
                }
            });

            *database_memmap.borrow_mut() = Some(db);

            button_export.set_visible(true);

            let b = database_memmap.borrow();
            let mmap = &b.as_ref().unwrap().mmap[..];
            let db: Database = Database::new(mmap);

            let tables = db.tables();
            for table in tables.iter() {
                add_table_row(table);
            }
            listbox.show_all();

            let widget = listbox.get_row_at_index(0);
            listbox.select_row(widget.as_ref());
        }
    };

    button_open.connect_clicked({
        let window = window.clone();
        move |_| {
            let file_chooser = gtk::FileChooserDialog::new(
                Some("Foo"),
                Some(&window),
                gtk::FileChooserAction::Open,
            );
            file_chooser.add_button("_Cancel", gtk::ResponseType::Cancel);
            file_chooser.add_button("_Open", gtk::ResponseType::Accept);
            let filter = gtk::FileFilter::new();
            filter.add_pattern("*.fdb");
            filter.set_name(Some("FDB-Files"));
            file_chooser.set_filter(&filter);

            match file_chooser.run() {
                gtk::ResponseType::Accept => {
                    let file = file_chooser.get_filename().unwrap();
                    match try_load_file(&file) {
                        Ok(db) => {
                            header.set_subtitle(file.to_str());
                            load(db);
                        }
                        Err(err) => {
                            let msg = gtk::MessageDialogBuilder::new()
                                .text(&format!("Could not open database file: {}", err))
                                .buttons(gtk::ButtonsType::Ok)
                                .build();
                            msg.run();
                            msg.close();
                        }
                    }
                }
                gtk::ResponseType::Cancel => {
                    println!("Cancel");
                }
                _ => {}
            }
            file_chooser.close();
        }
    });

    button_export.connect_clicked({
        let window = window.clone();
        move |_| {
            let file_chooser = gtk::FileChooserDialog::new(
                Some("Foo"),
                Some(&window),
                gtk::FileChooserAction::Save,
            );
            file_chooser.add_button("_Cancel", gtk::ResponseType::Cancel);
            file_chooser.add_button("_Save", gtk::ResponseType::Accept);

            file_chooser.set_do_overwrite_confirmation(true);
            file_chooser.set_current_name("export.sqlite");

            let filter = gtk::FileFilter::new();
            filter.add_pattern("*.sqlite");
            filter.add_pattern("*.db");
            filter.set_name(Some("SQLite-Files"));
            file_chooser.set_filter(&filter);

            match file_chooser.run() {
                gtk::ResponseType::Accept => {
                    let file = file_chooser.get_filename().unwrap();
                    println!("{}", file.display());

                    let b = database_memmap.borrow();
                    let mmap = &b.as_ref().unwrap().mmap[..];
                    let db: Database = Database::new(mmap);

                    try_export_db(&file, db).unwrap();
                }
                gtk::ResponseType::Cancel => {
                    println!("Cancel");
                }
                _ => {}
            }
            file_chooser.close();
        }
    });

    window.connect_destroy(|_| gtk::main_quit());

    window.show_all();
    gtk::main();
}
