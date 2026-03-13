use anyhow::Result;
use std::sync::Arc;
use tracing::info;

use crate::{
    backfill::BackfillController,
    config::Config,
    control::RuntimeControl,
    crawler::Crawler,
    db::Database,
    llm::OpenAiCompatibleClient,
    settings::SettingsManager,
    ui::{self, UiContext},
};

pub fn run(config: Config) -> Result<()> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("hunger")
            .build()?,
    );

    let settings = runtime.block_on(SettingsManager::new(config))?;
    let active_config = runtime.block_on(settings.current());
    let database = runtime.block_on(Database::connect(&active_config.database_url))?;
    runtime.block_on(database.enqueue_seed_urls(&active_config.seed_urls))?;

    let llm = OpenAiCompatibleClient::new(&active_config)?;
    info!(
        llm_base_url = %active_config.llm_base_url,
        llm_model = %active_config.llm_model,
        embedding_model = ?active_config.embedding_model,
        page_semantic_map_enabled = active_config.page_semantic_map_enabled,
        llm_semantic_map_enabled = active_config.llm_semantic_map_enabled,
        settings_path = %active_config.settings_path,
        auth_mode = llm.auth_mode(),
        "starting Hunger",
    );

    let control = RuntimeControl::default();
    let crawler = Crawler::new(settings.clone(), database.clone(), control.clone())?;
    runtime.spawn(crawler.run_forever());
    let backfill = BackfillController::default();

    let context = UiContext {
        runtime,
        database,
        settings,
        control,
        backfill,
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1660.0, 960.0])
            .with_min_inner_size([1180.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Hunger",
        options,
        Box::new(move |cc| Ok(Box::new(ui::HungerApp::new(cc, context.clone())))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}
