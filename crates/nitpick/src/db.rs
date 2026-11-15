use std::future::Future;

use gpui::{App, Global};

struct AppDb(app_db::Db);

impl Global for AppDb {}

pub fn init(cx: &mut App) {
    let path = crate::settings_store::app_dir(dirs::data_dir()).join("nitpick.db");
    let db = app_db::Db::open(&path).or_else(|err| {
        log::error!(
            "opening {}: {err}; nothing is saved this session",
            path.display()
        );
        app_db::Db::open_in_memory()
    });
    match db {
        Ok(db) => cx.set_global(AppDb(db)),
        Err(err) => log::error!("in-memory database: {err}"),
    }
}

pub fn db(cx: &App) -> Option<app_db::Db> {
    cx.try_global::<AppDb>().map(|global| global.0.clone())
}

pub fn barrier(cx: &App) -> impl Future<Output = ()> + 'static + use<> {
    let db = db(cx);
    async move {
        if let Some(db) = db
            && let Err(err) = db.barrier().await
        {
            log::warn!("finishing database writes: {err}");
        }
    }
}
