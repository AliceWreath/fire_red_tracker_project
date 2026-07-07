//! Router construction and server entry point.

use super::*;

// ---------------------------------------------------------------------------
// Router construction (extracted for testability)
// ---------------------------------------------------------------------------

pub(crate) fn build_router(web_state: WebState) -> Router {
    Router::new()
        .route("/", get(serve_login_page))
        .route("/static/overlay.js", get(serve_overlay_js))
        .route("/overlay", get(serve_html))
        .route("/ws", get(ws_handler))
        .route("/db", get(serve_db_viewer))
        .route("/db.json", get(serve_db_json))
        .route("/db/clear", post(clear_db))
        .route("/db/query", get(serve_db_query))
        .route("/cmd", get(serve_cmd))
        .route("/api/state", get(api_state))
        .route("/api/slot/:index", get(api_slot))
        .route("/api/slot/:index/odds", get(api_slot_odds))
        .route("/api/slot/:index/give_item", post(api_give_item))
        .route("/api/slot/:index/take_item", post(api_take_item))
        .route("/api/slot/:index/change_species", post(api_change_species))
        .route("/api/slot/:index/change_ability", post(api_change_ability))
        .route("/api/slot/:index/change_gender", post(api_change_gender))
        .route("/api/slot/:index/make_shiny", post(api_make_shiny))
        .route(
            "/api/slot/:index/change_nickname",
            post(api_change_nickname),
        )
        .route(
            "/api/slot/:index/change_held_item",
            post(api_change_held_item),
        )
        .route("/api/slot/:index/cure_status", post(api_cure_status))
        .route("/api/slot/:index/change_nature", post(api_change_nature))
        .route("/api/slot/:index/restore_pp", post(api_restore_pp))
        .route("/api/slot/:index/set_friendship", post(api_set_friendship))
        .route("/api/slot/:index/change_move", post(api_change_move))
        .route("/api/slot/:index/set_ivs", post(api_set_ivs))
        .route("/api/slot/:index/increase_ivs", post(api_increase_ivs))
        .route("/api/slot/:index/set_evs", post(api_set_evs))
        .route("/api/slot/:index/increase_evs", post(api_increase_evs))
        .route("/api/slot/:index/restore_hp", post(api_restore_hp))
        .route("/api/slot/:index/heal_party", post(api_heal_party))
        .route("/api/slot/:index/set_exp", post(api_set_exp))
        .route("/api/slot/:index/set_level", post(api_set_level))
        .route("/api/slot/:index/learn_move", post(api_learn_move))
        .route("/api/slot/:index/forget_move", post(api_forget_move))
        .route("/api/slot/:index/set_pokerus", post(api_set_pokerus))
        .route("/api/slot/:index/set_pp_ups", post(api_set_pp_ups))
        .route("/api/slot/:index/revive_pokemon", post(api_revive_pokemon))
        .route("/api/slot/:index/undo", post(api_undo))
        .route("/api/slot/:index/refresh_rom", post(api_refresh_rom))
        .route("/api/bot/:index", get(api_bot_summary))
        .route("/api/command/:cmd", post(api_command))
        .route("/api/db/query", post(api_db_query))
        .route("/api/runs", get(api_runs))
        .route("/api/run/import", post(api_run_import))
        .route("/api/run/:id/stats", get(api_run_stats))
        .route("/api/run/:id/route_stats", get(api_run_route_stats))
        .route("/api/run/:id/route_odds", get(api_run_route_odds))
        .route("/api/run/:id/webhook_log", get(api_run_webhook_log))
        .route(
            "/api/run/:id/soul_link/overrides",
            get(api_run_soul_link_overrides),
        )
        .route(
            "/api/run/:id/soul_link/override",
            post(api_set_soul_link_override),
        )
        .route(
            "/api/run/:id/soul_link/override/:personality",
            delete(api_clear_soul_link_override),
        )
        .route("/api/run/:id/shiny", get(api_shiny_stats))
        .route("/api/run/:id/export", get(api_run_export))
        .route("/api/run/:id/events", get(api_run_events))
        .route("/api/timeline", get(api_active_timeline))
        .route("/history", get(serve_history))
        .route("/shiny", get(serve_shiny))
        .route("/memorial", get(serve_memorial))
        .route("/soullink", get(serve_soullink))
        .route("/soullink/manage", get(serve_soullink_manage))
        .route("/alerts", get(serve_alerts))
        .route("/:index/alerts", get(serve_alerts))
        .route("/:index/routes", get(serve_routes))
        .route("/:index/party", get(serve_party))
        .route("/:index/encounters", get(serve_focused))
        .route("/:index/dead", get(serve_focused))
        .route("/:index/caught", get(serve_focused))
        .route("/:index/box", get(serve_focused))
        .route("/:index/types", get(serve_types_page))
        .route("/:index/items", get(serve_items))
        .route("/:index/moves", get(serve_moves_page))
        .route("/api/slot/:index/bag", get(api_bag))
        .route("/run/:id/stats", get(serve_run_stats))
        .route("/run/:id/memorial", get(serve_memorial))
        .route("/run/:id/timeline", get(serve_timeline))
        .route("/party/mobile", get(serve_mobile_party))
        .route("/timeline", get(serve_timeline))
        .route("/species", get(serve_species))
        .route("/api/species/stats", get(api_species_stats))
        .route("/trainers", get(serve_trainers))
        .route("/run/:id/trainers", get(serve_trainers))
        .route("/api/run/:id/trainers", get(api_run_trainers))
        .route("/api/runs/compare", get(api_runs_compare))
        .route("/api/run/:id/luck", get(api_run_luck))
        .route("/api/run/:id/closest_calls", get(api_run_closest_calls))
        .route("/api/catch_rate", get(api_catch_rate))
        .route(
            "/api/run/:id/pokemon/:personality/hp_history",
            get(api_run_pokemon_hp_history),
        )
        .route("/api/run/:id/enemy_hp_log", get(api_run_enemy_hp_log))
        .route("/api/run/:id/battle_damage", get(api_run_battle_damage))
        .route("/api/run/:id/summary", get(api_run_summary))
        .route("/api/run/:id/report", get(api_run_report))
        .route(
            "/api/run/:id/event/:event_id/note",
            patch(api_set_event_note).delete(api_clear_event_note),
        )
        .route("/api/run/:id/pokepaste", get(api_run_pokepaste))
        .route("/api/run/:id/splits", get(api_run_splits))
        .route("/api/run/:id/catch_log", get(api_run_catch_log))
        .route("/api/run/:id/difficulty", get(api_run_difficulty))
        .route("/api/run/:id/area_times", get(api_run_area_times))
        .route("/api/run/:id/death_map", get(api_run_death_map))
        .route("/api/run/:id/level_curve", get(api_run_level_curve))
        .route("/api/run/:id/move_usage", get(api_run_move_usage))
        .route("/api/run/:id/friendship", get(api_run_friendship))
        .route("/api/slot/:index/ev_progress", get(api_slot_ev_progress))
        .route("/:index/deaths", get(serve_deaths_overlay))
        .route("/:index/encounter_count", get(serve_encounter_count))
        .route("/:index/hp", get(serve_hp_overlay))
        .route("/:index/damage_calc", get(serve_damage_calc_overlay))
        .route("/:index/badges", get(serve_badges_overlay))
        .route("/:index/nextgym", get(serve_next_gym_overlay))
        .route("/:index/encounter_table", get(serve_encounter_table_overlay))
        .route("/:index/money", get(serve_money_overlay))
        .route("/:index/playtime", get(serve_playtime_overlay))
        .route("/:index/goals", get(serve_goals_overlay))
        .route("/:index/vs_leader", get(serve_vs_leader_overlay))
        .route("/:index/goals/manage", get(serve_goals_manage))
        .route("/api/run/:id/goals", get(api_list_run_goals))
        .route("/api/goal", post(api_post_goal))
        .route("/api/goal/:id/complete", patch(api_complete_goal))
        .route("/api/goal/:id", patch(api_set_goal_completed).delete(api_delete_goal))
        .route("/api/slot/:index/command/:cmd", post(api_slot_command))
        .route("/about", get(serve_about))
        .route("/guide", get(serve_guide_page))
        .route("/mobile", get(serve_mobile_page))
        .route("/compare", get(serve_compare))
        .route("/join", get(serve_join))
        .route("/register", get(serve_register))
        .route("/api/direct/connect", post(api_direct_connect).delete(api_direct_disconnect))
        .route("/api/direct/hosts", get(api_direct_hosts))
        .route("/api/run", post(api_create_run))
        .route("/api/run/:id/resume", post(api_resume_run))
        // Batch injection
        .route("/api/batch", post(api_batch_inject))
        // Presets
        .route("/api/preset", post(api_save_preset))
        .route("/api/presets", get(api_list_presets))
        .route("/api/preset/:name", delete(api_delete_preset))
        .route("/api/preset/:name/apply", post(api_apply_preset))
        // Challenge rules
        .route("/api/run/:id/rules", get(api_get_run_rules).patch(api_patch_run_rules))
        // Display column order
        .route("/api/run/:id/player_slots", get(api_get_run_player_slots).patch(api_patch_run_player_slots))
        // Per-section CSV exports
        .route("/api/run/:id/encounters.csv", get(api_run_encounters_csv))
        .route("/api/run/:id/deaths.csv", get(api_run_deaths_csv))
        .route("/api/run/:id/events.csv", get(api_run_events_csv))
        // Discord slash-command interactions endpoint
        .route("/interactions", post(discord_interactions))
        // Analytics: type usage heatmap, ghost run comparison, shiny pressure, status log
        .route("/api/run/:id/type_matchups", get(api_run_type_matchups))
        .route("/api/run/:id/vs/:ghost_id", get(api_run_ghost_compare))
        .route("/api/slot/:index/shiny_pressure", get(api_slot_shiny_pressure))
        .route("/api/run/:id/status_log", get(api_run_status_log))
        .route("/api/run/:id/dex", get(api_run_dex))
        // Share URL
        .route("/api/run/:id/share", post(api_create_share))
        .route("/share/:token/state", get(api_share_state))
        // Config hot-reload
        .route("/api/config/reload", post(api_config_reload))
        // Manual full-database backup (owner-only)
        .route("/api/backup", post(api_backup_now))
        // Donation/alert trigger bridge
        .route("/api/webhook/donation", post(api_donation_webhook))
        // Savefile snapshot import
        .route("/api/savefile", post(api_import_savefile))
        // User accounts
        .route("/api/users", post(api_register_user).get(api_list_users))
        .route("/api/login", post(api_login))
        .route("/api/logout", post(api_logout))
        .route("/api/me", get(api_me))
        .route("/api/me/sessions", get(api_my_sessions))
        .route("/api/me/sessions/revoke_others", post(api_revoke_other_sessions))
        .route("/api/me/sessions/:prefix", delete(api_revoke_session))
        .route("/api/me/token", get(api_me_token))
        .route("/api/me/dashboard", get(api_me_dashboard))
        .route("/api/me/active_run", get(api_me_active_run).put(api_me_set_active_run))
        .route("/api/me/integrations", get(api_get_integrations))
        .route("/api/me/integrations/:kind", axum::routing::put(api_put_integration).delete(api_delete_integration))
        .route("/api/user/:id/runs", get(api_user_runs))
        // Run invites and access requests
        .route("/api/run/:id/end", post(api_end_run))
        .route("/api/run/:id/invite", post(api_run_invite))
        .route("/api/run/:id/invites", get(api_run_invites))
        .route("/api/run/:id/invite/accept", post(api_run_invite_accept))
        .route("/api/run/:id/invite/decline", post(api_run_invite_decline))
        .route("/api/run/:id/invite/request", post(api_run_invite_request))
        .route("/api/run/:id/invite/requests", get(api_run_invite_requests))
        .route("/api/run/:id/invite/request/:uid/approve", post(api_run_invite_request_approve))
        .route("/api/run/:id/invite/request/:uid/deny", post(api_run_invite_request_deny))
        .route("/api/me/run_statuses", get(api_me_run_statuses))
        .route("/api/me/run_requests", get(api_me_run_requests))
        // Dashboard + integrations pages
        .route("/dashboard", get(serve_dashboard))
        .route("/integrations", get(serve_integrations_page))
        // Overlays
        .route("/:index/dex", get(serve_dex_overlay))
        .route("/:index/typechart", get(serve_typechart_overlay))
        // ── Middleware stack (last added = outermost = runs first) ──────────
        // 4. Slot access: check ownership before any request to /api/slot/:idx/…
        .layer(axum::middleware::from_fn_with_state(
            web_state.clone(),
            slot_access_middleware,
        ))
        // 3. Run access: check user_can_access_run for /api/run/:id/… routes
        .layer(axum::middleware::from_fn(run_access_middleware))
        // 2. Auth wall: require a valid session for all non-public routes
        .layer(axum::middleware::from_fn(auth_middleware))
        // 1. Display-slot rewrite: translate /<n>/… page URLs from a pinned
        //    display slot to the physical live-slot index, before auth/access
        //    checks or routing see the path. Runs first since it's outermost.
        .layer(axum::middleware::from_fn_with_state(
            web_state.clone(),
            slot_display_index_middleware,
        ))
        .with_state(web_state)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Optional configuration passed to [`run`] — bundles flags that are not
/// needed by every call site.
pub struct WebRunConfig {
    pub db_conn: Option<String>,
    pub testing: bool,
    pub allow_injections: bool,
    pub connector: Option<Arc<crate::direct::DirectConnector>>,
    pub backup_dir: Option<String>,
    pub backup_keep: usize,
    pub livesplit_split_on_badges: bool,
    pub discord_slash: Option<crate::config::DiscordSlashConfig>,
    /// Optional path to the TOML config file for config hot-reload support.
    pub config_path: Option<String>,
}

pub fn run(live_slots: SharedSlots, port: u16, cfg: WebRunConfig) {
    let WebRunConfig {
        db_conn,
        testing,
        allow_injections,
        connector,
        backup_dir,
        backup_keep,
        livesplit_split_on_badges,
        discord_slash,
        config_path,
    } = cfg;
    let sprites: PngSpriteCache = Arc::new(Mutex::new(HashMap::new()));

    // Wire the shared sprite cache into any already-connected slots and keep
    // it available for slots that connect later (BroadcastLoop sets it on drain).
    {
        let slots = live_slots.lock_or_recover();
        for slot in slots.iter() {
            *slot.sprite_cache.lock_or_recover() = Some(sprites.clone());
        }
    }

    let (tx, _rx) = watch::channel::<String>(String::new());
    let tx_bg = tx.clone();
    let sprites_loop = sprites.clone();
    let loop_slots = live_slots.clone();
    let loop_db = db_conn.clone();
    let loop_backup_dir = backup_dir.clone();

    std::thread::spawn(move || {
        let mut bloop = BroadcastLoop::new(
            loop_slots,
            sprites_loop,
            loop_db,
            loop_backup_dir,
            livesplit_split_on_badges,
        );
        loop {
            if let Some(json) = bloop.tick() {
                let _ = tx_bg.send(json);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    let web_state = WebState {
        tx,
        live_slots,
        db_conn,
        testing,
        allow_injections,
        connector,
        backup_dir,
        backup_keep,
        discord_slash,
        config_path: config_path.map(Arc::new),
        user_active_run: Arc::new(Mutex::new(HashMap::new())),
        integration_manager: Arc::new(Mutex::new(HashMap::new())),
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let app = build_router(web_state);

        let addr = format!("0.0.0.0:{}", port);
        tracing::info!("WebSocket overlay listening on http://{}", addr);
        tracing::info!("Add in OBS as Browser Source: http://localhost:{}", port);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("failed to bind WebSocket port");
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            tracing::error!("WebSocket server error: {e}");
        }
    });
}
