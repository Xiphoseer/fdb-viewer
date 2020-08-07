extern crate gio;
extern crate gtk;
#[macro_use]
extern crate glib;

use assembly_data::fdb::align::Database;
use gio::prelude::*;
use glib::subclass::types::ObjectSubclass;
use gtk::prelude::*;
use memmap::Mmap;
use std::{cell::RefCell, fs::File, io, path::PathBuf, rc::Rc};

mod table;
use table::SimpleObject;

struct DB {
    //_file: File,
    mmap: Mmap,
}

fn try_load_file(path: PathBuf) -> io::Result<DB> {
    let _file = File::open(&path)?;
    let mmap = unsafe { Mmap::map(&_file)? };
    Ok(DB { /*_file,*/ mmap, })
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
    let pane_container: gtk::Paned = builder.get_object("paned1").unwrap();

    window
        .get_style_context()
        .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_FALLBACK);

    let button_open: gtk::Button = builder.get_object("button-open").unwrap();

    /*let hsize_group = gtk::SizeGroupBuilder::new()
    .mode(gtk::SizeGroupMode::Horizontal)
    .build();*/

    let left_box = gtk::BoxBuilder::new()
        .orientation(gtk::Orientation::Vertical)
        .build();

    let entry = gtk::SearchEntryBuilder::new()
        .placeholder_text("Search table...")
        .build();

    entry.connect_search_changed(|_| {
        // TODO
    });

    let searchbar = gtk::SearchBar::new();
    searchbar.add(&entry);
    searchbar.set_hexpand(false);

    let listbox = gtk::ListBox::new();
    listbox.get_style_context().add_class("tweak-categories");
    listbox.set_size_request(250, -1);
    listbox.connect_row_selected(|_list, obj| {
        if let Some(row) = obj {
            let label: gtk::Label = row.get_child().unwrap().downcast().unwrap();
            let name = label.get_text();
            println!("Selected row {}", name);
        } else {
            println!("Unselect Row")
        }
    });

    listbox.set_header_func(Some(Box::new(|row: &gtk::ListBoxRow, before| {
        if before.is_some() && row.get_header().is_none() {
            let sep = gtk::SeparatorBuilder::new()
                .orientation(gtk::Orientation::Horizontal)
                .build();
            row.set_header(Some(&sep));
        }
    })));

    let table_model = gio::ListStore::new(SimpleObject::get_type());
    listbox.bind_model(Some(&table_model), |obj| {
        let name_prop = obj.get_property("name").unwrap();
        let name = name_prop.get::<&str>().unwrap().unwrap();
        let btn = gtk::LabelBuilder::new()
            .label(name)
            .halign(gtk::Align::Start)
            .margin(5)
            .build();
        btn.upcast()
    });

    let scroll = gtk::ScrolledWindowBuilder::new()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scroll.add(&listbox);

    left_box.pack_start(&searchbar, false, false, 0);
    left_box.pack_start(&scroll, true, true, 0);

    //hsize_group.add_widget(&left_box);

    let tree = gtk::TreeViewBuilder::new().build();

    pane_container.add1(&left_box);
    pane_container.set_child_shrink(&left_box, false);
    pane_container.add2(&tree);

    let w = window.clone();
    let m = database_memmap.clone();
    let tm = table_model.clone();
    button_open.connect_clicked(move |_| {
        let file_chooser =
            gtk::FileChooserDialog::new(Some("Foo"), Some(&w), gtk::FileChooserAction::Open);
        file_chooser.add_button("_Cancel", gtk::ResponseType::Cancel);
        file_chooser.add_button("_Open", gtk::ResponseType::Accept);
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.fdb");
        filter.set_name(Some("FDB-Files"));
        file_chooser.set_filter(&filter);
        match file_chooser.run() {
            gtk::ResponseType::Accept => {
                let file = file_chooser.get_filename().unwrap();
                println!("Accept: {}", file.display());

                match try_load_file(file) {
                    Ok(db) => {
                        *m.borrow_mut() = Some(db);

                        let b = m.borrow();
                        let mmap = &b.as_ref().unwrap().mmap[..];
                        let db: Database = Database::new(mmap);

                        let tables = db.tables();
                        for table in tables.iter() {
                            let name = table.name();
                            let n = name.as_ref();

                            let so = glib::Object::new(SimpleObject::get_type(), &[]).unwrap();
                            so.set_property("name", &n).unwrap();

                            tm.append(&so);
                        }

                        /*let msg = gtk::MessageDialogBuilder::new()
                            .text(&format!("Opened database file with {} tables", len))
                            .buttons(gtk::ButtonsType::Ok)
                            .build();
                        msg.run();
                        msg.close();*/
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
                println!("Accept");
            }
            gtk::ResponseType::Close => {
                println!("Close");
            }
            _ => {
                println!("Other");
            }
        }
        file_chooser.close();
    });
    window.connect_destroy(|_| gtk::main_quit());

    window.show_all();
    gtk::main();
}
