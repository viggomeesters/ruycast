//! Rust host domain and persistence, without native handles or UI startup.
pub mod artifact_lease;
pub mod actions;
pub mod activity;
pub mod analytics;
pub mod analytics_dashboard;
pub mod caps_io;
pub mod caps_lock;
pub mod carbon_hotkeys;
pub mod catalog;
pub mod clipboard_ownership;
pub mod clipboard_history;
pub mod clipboard_image;
pub mod command_identity;
pub mod config_effects;
pub mod configuration_jobs;
pub mod connected;
pub mod control;
mod deadline;
pub mod execution;
mod hid_io;
pub mod hid_service;
pub mod hotkeys;
pub mod keyboard_service;
pub mod keyboard_state;
pub mod keyword_capture;
pub mod keyword_effects;
mod keyword_io;
pub mod keyword_matcher;
pub mod keyword_target;
pub mod native_text;
pub mod observe;
#[cfg(feature = "test-tools")]
pub mod owned_host_entry;
pub mod presentation;
pub mod profile;
pub mod profile_io_abi;
pub mod profile_io;
pub mod permissions;
pub mod performance_trace;
mod performance_signposts;
pub mod ranking;
pub mod recorder;
mod request_values;
pub mod runtime;
pub mod runtime_health;
pub mod session;
pub mod state;
pub mod startup;
pub mod system_search;
pub mod supervision;
pub mod workspace_activation;
// One reviewed implementation serves the existing CLI and new host.
#[path = "../../rust/src/private_storage.rs"]
pub mod private_storage;

pub mod creation;

pub mod library;
pub mod management_commands;
pub mod settings_surface;
pub mod menu_bar;

pub mod script_result;

pub mod script_session;

pub mod app_choice;
pub mod applications;

pub mod utilities;
pub mod utility_storage;
pub mod utility_jobs;
pub mod workspace;
pub mod window_placement;
pub mod window_layout;
pub mod os_command;
pub mod file_search;
pub mod processes;
pub mod resource_sampler;
pub mod monitor;
pub mod management_rest;
pub mod system_actions;
pub mod builtin_effects;

pub mod main_services;
pub mod templates;

pub mod template_form;

pub mod paste;
pub mod snippet_choice;
pub mod snippet_choice_request;
pub mod snippet_clipboard;
pub mod snippet_output;

pub mod responder_edit;
#[path = "native_snippet_choice.rs"]
pub mod snippet_choice_window;

pub mod clipboard_preview;
pub mod website_icons;
pub mod task_date;
#[cfg(feature = "test-tools")]
pub mod native_f1;
#[cfg(feature = "test-tools")]
pub mod native_callback_roster;
