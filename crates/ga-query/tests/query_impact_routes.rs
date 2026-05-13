//! Tools S-006 cluster C5 — AS-014 framework-aware route detection.
//!
//! Covers 5 frameworks spec'd by AS-014: gin, Django urls.py, Rails routes.rb,
//! axum Router, nest @Controller. Plus negative tests: non-handler symbol →
//! empty; non-route file → not scanned.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn run(store: &Store, symbol: &str) -> ga_query::ImpactResponse {
    impact(
        store,
        &ImpactRequest {
            symbol: Some(symbol.into()),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn gin_post_route_detected_for_handler() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handler.go"),
        "package main\n\nfunc CreateUser() {}\n",
    );
    write(
        &repo.join("routes.go"),
        "package main\n\nfunc setup(r *gin.Engine) {\n    \
             r.POST(\"/api/users\", CreateUser)\n    \
             r.GET(\"/health\", Health)\n\
         }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "CreateUser");
    assert_eq!(resp.affected_routes.len(), 1, "{:?}", resp.affected_routes);
    let r = &resp.affected_routes[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/users");
    assert_eq!(r.source_file, "routes.go");
}

#[test]
fn gin_handler_with_receiver_method_ref() {
    // r.GET("/users/:id", h.GetUser) → handler name = GetUser.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handler.go"),
        "package main\n\ntype UserHandler struct{}\n\nfunc (h *UserHandler) GetUser() {}\n",
    );
    write(
        &repo.join("routes.go"),
        "package main\n\nfunc setup(r *gin.Engine, h *UserHandler) {\n    \
             r.GET(\"/users/:id\", h.GetUser)\n\
         }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "GetUser");
    assert_eq!(resp.affected_routes.len(), 1);
    assert_eq!(resp.affected_routes[0].method, "GET");
    assert_eq!(resp.affected_routes[0].path, "/users/:id");
}

#[test]
fn django_urls_path_routes_detected() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("views.py"), "def login_view():\n    pass\n");
    write(
        &repo.join("urls.py"),
        "from django.urls import path\nfrom . import views\n\n\
         urlpatterns = [\n    \
             path('login/', views.login_view, name='login'),\n    \
             path('logout/', views.logout_view),\n\
         ]\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "login_view");
    assert_eq!(resp.affected_routes.len(), 1, "{:?}", resp.affected_routes);
    let r = &resp.affected_routes[0];
    assert_eq!(r.path, "login/");
    // Django path() has no method binding — report ANY.
    assert_eq!(r.method, "ANY");
    assert_eq!(r.source_file, "urls.py");
}

#[test]
fn rails_routes_detected() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Ruby isn't parsed into the graph (see TODO.md), but file node exists
    // with path "config/routes.rb" and we still read raw bytes for routes.
    // Since ga-parser doesn't handle Ruby, File node for .rb may not be
    // created — write the file but also a dummy .py so indexer has a file.
    write(&repo.join("app.py"), "def create(): pass\n");
    write(
        &repo.join("config/routes.rb"),
        "Rails.application.routes.draw do\n  \
             get '/api/users' => 'users#index'\n  \
             post '/api/users' => 'users#create'\n\
         end\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "create");
    let hits: Vec<_> = resp
        .affected_routes
        .iter()
        .filter(|r| r.source_file == "config/routes.rb")
        .collect();
    assert!(!hits.is_empty(), "{:?}", resp.affected_routes);
    assert_eq!(hits[0].method, "POST");
    assert_eq!(hits[0].path, "/api/users");
}

#[test]
fn axum_router_route_post_detected() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handlers.rs"),
        "pub fn create_user() {}\npub fn list_users() {}\n",
    );
    write(
        &repo.join("server.rs"),
        "use axum::{Router, routing::{get, post}};\nuse crate::handlers::*;\n\n\
         fn app() -> Router {\n    \
             Router::new()\n        \
                 .route(\"/users\", post(create_user))\n        \
                 .route(\"/users\", get(list_users))\n\
         }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "create_user");
    assert_eq!(resp.affected_routes.len(), 1);
    assert_eq!(resp.affected_routes[0].method, "POST");
    assert_eq!(resp.affected_routes[0].path, "/users");
    assert_eq!(resp.affected_routes[0].source_file, "server.rs");
}

#[test]
fn nest_controller_post_method_detected() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("users.controller.ts"),
        "import { Controller, Post, Body } from '@nestjs/common';\n\n\
         @Controller('/users')\n\
         export class UsersController {\n    \
             @Post()\n    \
             async createUser(@Body() dto: any) { return dto; }\n\n    \
             @Get(':id')\n    \
             async getUser(@Param('id') id: string) { return id; }\n\
         }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "createUser");
    assert_eq!(resp.affected_routes.len(), 1, "{:?}", resp.affected_routes);
    let r = &resp.affected_routes[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/users");
    assert_eq!(r.source_file, "users.controller.ts");
}

#[test]
fn nest_method_decorator_path_joined_with_controller_prefix() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("users.controller.ts"),
        "import { Controller, Get } from '@nestjs/common';\n\n\
         @Controller('/api/users')\n\
         export class UsersController {\n    \
             @Get('/:id')\n    \
             getUser(@Param('id') id: string) { return id; }\n\
         }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "getUser");
    assert_eq!(resp.affected_routes.len(), 1);
    assert_eq!(resp.affected_routes[0].path, "/api/users/:id");
}

#[test]
fn unrelated_symbol_yields_no_routes() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("routes.go"),
        "package main\nfunc setup(r *gin.Engine) {\n    r.POST(\"/x\", Other)\n}\n",
    );
    write(
        &repo.join("handler.go"),
        "package main\nfunc CreateUser() {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "CreateUser");
    assert!(
        resp.affected_routes.is_empty(),
        "{:?}",
        resp.affected_routes
    );
}

#[test]
fn non_ident_seed_yields_no_routes() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("routes.go"),
        "package main\nfunc setup(r *gin.Engine) {\n    r.POST(\"/x\", CreateUser)\n}\n",
    );
    write(
        &repo.join("handler.go"),
        "package main\nfunc CreateUser() {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "Create'User");
    assert!(resp.affected_routes.is_empty());
}

#[test]
fn routes_deduped_and_sorted() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Same handler mounted twice on different methods.
    write(
        &repo.join("handler.go"),
        "package main\nfunc CreateUser() {}\n",
    );
    write(
        &repo.join("z_routes.go"),
        "package main\nfunc z() { r.POST(\"/b\", CreateUser) }\n",
    );
    write(
        &repo.join("a_routes.go"),
        "package main\nfunc a() { r.POST(\"/a\", CreateUser) }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "CreateUser");
    assert_eq!(resp.affected_routes.len(), 2);
    let paths: Vec<_> = resp
        .affected_routes
        .iter()
        .map(|r| r.path.clone())
        .collect();
    // Deterministic: /a before /b regardless of which file was scanned first.
    assert_eq!(paths, vec!["/a".to_string(), "/b".to_string()]);
}
