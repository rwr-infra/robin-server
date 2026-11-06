#[cfg(test)]
mod tests {
    use crate::{Announcement, Config, MapEntry, MapsConfig, RepoVersion, VersionInfo};
    use std::sync::Mutex;

    /// Serializes tests that read or write process environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn test_config_default_values() {
        let _env_guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Remove environment variables to test defaults
        unsafe {
            std::env::remove_var("HOST");
            std::env::remove_var("PORT");
            std::env::remove_var("CACHE_DURATION_SECS");
            std::env::remove_var("MAPS_CONFIG");
            std::env::remove_var("ANDROID_REPO_URL");
            std::env::remove_var("WEB_REPO_URL");
            std::env::remove_var("ANNOUNCEMENT_PATH");
            std::env::remove_var("MCP_HOST");
            std::env::remove_var("MCP_PORT");
            std::env::remove_var("MCP_ALLOWED_HOSTS");
        }

        let config = Config::new().unwrap();

        // Test default values
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, "5800");
        assert_eq!(config.cache_duration_secs, 3);
        assert_eq!(config.maps_config_path, "maps.json");
        assert!(config.android_repo_url.is_none());
        assert!(config.web_repo_url.is_none());
        assert!(config.announcement_path.is_none());
        assert_eq!(config.mcp_host, crate::DEFAULT_MCP_HOST);
        assert_eq!(config.mcp_port, Some(crate::DEFAULT_MCP_PORT));
        assert!(config.mcp_allowed_hosts.is_none());
    }

    #[tokio::test]
    async fn test_config_reads_mcp_environment_without_inheriting_host() {
        let _env_guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        unsafe {
            std::env::set_var("HOST", "0.0.0.0");
            std::env::remove_var("MCP_HOST");
            std::env::set_var("MCP_PORT", "0");
            std::env::set_var("MCP_ALLOWED_HOSTS", "a.com, b.com");
        }

        let config = Config::new().unwrap();

        // Restore the environment before asserting so a failure cannot leak
        // state into other tests.
        unsafe {
            std::env::remove_var("HOST");
            std::env::remove_var("MCP_HOST");
            std::env::remove_var("MCP_PORT");
            std::env::remove_var("MCP_ALLOWED_HOSTS");
        }

        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.mcp_host, crate::DEFAULT_MCP_HOST);
        assert_ne!(config.mcp_host, config.host);
        assert_eq!(config.mcp_port, None);
        assert_eq!(
            config.mcp_allowed_hosts,
            Some(vec!["a.com".to_string(), "b.com".to_string()])
        );
    }

    #[tokio::test]
    async fn test_config_normalizes_bracketed_ipv6_mcp_host() {
        let _env_guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        unsafe {
            std::env::set_var("MCP_HOST", "[::1]");
            std::env::remove_var("MCP_PORT");
        }

        let config = Config::new().unwrap();

        unsafe {
            std::env::remove_var("MCP_HOST");
            std::env::remove_var("MCP_PORT");
        }

        assert_eq!(config.mcp_host, "::1");
    }

    #[tokio::test]
    async fn test_announcement_disabled_default() {
        let announcement = Announcement::disabled();
        assert!(!announcement.enabled);
        assert_eq!(announcement.html, "");

        // Disabled announcements still serialize to the frontend contract.
        let json = serde_json::to_string(&announcement).unwrap();
        assert_eq!(json, r#"{"enabled":false,"html":""}"#);
    }

    #[tokio::test]
    async fn test_announcement_missing_file_disabled() {
        let announcement =
            Announcement::load_from_file("this/path/does/not/exist/announcement.html").await;
        assert!(!announcement.enabled);
        assert_eq!(announcement.html, "");
    }

    #[tokio::test]
    async fn test_announcement_loads_file_content() {
        let dir = std::env::temp_dir();
        let path = dir.join("rwrs_test_announcement.html");
        let html = "<div class=\"notice\">Hello</div>";
        tokio::fs::write(&path, html).await.unwrap();

        let announcement = Announcement::load_from_file(path.to_str().unwrap()).await;
        assert!(announcement.enabled);
        assert_eq!(announcement.html, html);

        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn test_announcement_empty_file_disabled() {
        let dir = std::env::temp_dir();
        let path = dir.join("rwrs_test_announcement_empty.html");
        tokio::fs::write(&path, "   \n\t  ").await.unwrap();

        let announcement = Announcement::load_from_file(path.to_str().unwrap()).await;
        assert!(!announcement.enabled);
        assert_eq!(announcement.html, "");

        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn test_maps_config_creation() {
        let config = MapsConfig::new();
        assert_eq!(config.get_maps().len(), 0);
    }

    #[tokio::test]
    async fn test_version_info_creation() {
        let version_info = VersionInfo {
            android: RepoVersion {
                version: Some("v2.0.0".to_string()),
                url: Some("https://github.com/example/android/releases/tag/v2.0.0".to_string()),
            },
            web: RepoVersion {
                version: None,
                url: None,
            },
        };

        // Test serialization
        let json = serde_json::to_string(&version_info).unwrap();
        let parsed: VersionInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.android.version, Some("v2.0.0".to_string()));
        assert_eq!(parsed.web.version, None);
    }

    #[tokio::test]
    async fn test_version_info_empty() {
        let version_info = VersionInfo {
            android: RepoVersion {
                version: None,
                url: None,
            },
            web: RepoVersion {
                version: None,
                url: None,
            },
        };

        let json = serde_json::to_string(&version_info).unwrap();
        let parsed: VersionInfo = serde_json::from_str(&json).unwrap();

        assert!(parsed.android.version.is_none());
        assert!(parsed.web.version.is_none());
    }

    #[tokio::test]
    async fn test_map_entry_creation() {
        let entry = MapEntry {
            name: "Test Map".to_string(),
            path: "media/packages/vanilla/maps/test".to_string(),
            image: "test.png".to_string(),
        };

        assert_eq!(entry.name, "Test Map");
        assert_eq!(entry.path, "media/packages/vanilla/maps/test");
        assert_eq!(entry.image, "test.png");
    }
}
