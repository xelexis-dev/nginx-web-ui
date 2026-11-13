// Root startup options apply before per-database libraries can load; nginx cannot set SUSET GUCs.
pub const ROOT_OPTIONS:&str="-c session_preload_libraries= -c local_preload_libraries= -c search_path=pg_catalog -c statement_timeout=3600000 -c lock_timeout=10000";
pub const APP_OPTIONS:&str="-c search_path=pg_catalog -c statement_timeout=3600000 -c lock_timeout=10000";
