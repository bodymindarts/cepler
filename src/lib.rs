#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![cfg_attr(feature = "fail-on-warnings", deny(clippy::all))]

mod concourse;
mod config;
mod database;
mod repo;
mod workspace;

pub mod cli;
