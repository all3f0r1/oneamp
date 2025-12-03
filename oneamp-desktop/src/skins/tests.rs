// Comprehensive tests for the skin system
// Tests cover loading, switching, persistence, and fallback scenarios

#[cfg(test)]
mod tests {
    use std::path::Path;
    use super::super::*;

    #[test]
    fn test_skin_discovery_and_loading() {
        // Test that skins can be discovered and loaded
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        // Should have at least 2 skins (OneAmp Dark and Winamp5 Classified)
        assert!(
            skin_manager.available_skins.len() >= 2,
            "Expected at least 2 skins, found {}",
            skin_manager.available_skins.len()
        );
        
        // Check that skins have valid metadata
        for skin in &skin_manager.available_skins {
            assert!(!skin.metadata.name.is_empty(), "Skin name should not be empty");
            assert!(!skin.metadata.author.is_empty(), "Skin author should not be empty");
            assert!(!skin.metadata.version.is_empty(), "Skin version should not be empty");
        }
    }

    #[test]
    fn test_default_skin_is_oneamp_dark() {
        // Test that the default skin is OneAmp Dark
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        let active_skin = skin_manager.get_active_skin();
        
        assert_eq!(
            active_skin.metadata.name, "OneAmp Dark",
            "Default skin should be OneAmp Dark"
        );
    }

    #[test]
    fn test_skin_switching() {
        // Test that skins can be switched
        let mut skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        let initial_index = skin_manager.active_skin_index;
        let initial_name = skin_manager.get_active_skin().metadata.name.clone();
        
        // Switch to a different skin if available
        if skin_manager.available_skins.len() > 1 {
            let new_index = (initial_index + 1) % skin_manager.available_skins.len();
            let result = skin_manager.set_active_skin(new_index);
            
            assert!(result, "Skin switching should succeed");
            assert_eq!(
                skin_manager.active_skin_index, new_index,
                "Active skin index should be updated"
            );
            
            let new_name = skin_manager.get_active_skin().metadata.name.clone();
            assert_ne!(
                initial_name, new_name,
                "Skin name should change after switching"
            );
        }
    }

    #[test]
    fn test_skin_by_name_lookup() {
        // Test that skins can be found by name
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        let index = skin_manager.find_skin_by_name("OneAmp Dark");
        assert!(index.is_some(), "OneAmp Dark skin should be found");
        
        let index = skin_manager.find_skin_by_name("NonExistentSkin");
        assert!(index.is_none(), "Non-existent skin should not be found");
    }

    #[test]
    fn test_skin_colors_are_valid() {
        // Test that skin colors are valid hex values
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        for skin in &skin_manager.available_skins {
            // Test background color
            assert!(
                skin.colors.background.starts_with('#'),
                "Background color should be hex format"
            );
            
            // Test text color
            assert!(
                skin.colors.text.starts_with('#'),
                "Text color should be hex format"
            );
            
            // Test accent color
            assert!(
                skin.colors.accent.starts_with('#'),
                "Accent color should be hex format"
            );
        }
    }

    #[test]
    fn test_skin_metrics_are_reasonable() {
        // Test that skin metrics have reasonable values
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        for skin in &skin_manager.available_skins {
            // Rounding should be between 0 and 20
            assert!(
                skin.metrics.window_rounding >= 0.0 && skin.metrics.window_rounding <= 20.0,
                "Window rounding should be between 0 and 20"
            );
            
            // Text sizes should be reasonable (8-72 points)
            assert!(
                skin.metrics.body_text_size >= 8.0 && skin.metrics.body_text_size <= 72.0,
                "Body text size should be between 8 and 72"
            );
            
            // Padding should be reasonable (0-50)
            assert!(
                skin.metrics.window_padding >= 0.0 && skin.metrics.window_padding <= 50.0,
                "Window padding should be between 0 and 50"
            );
        }
    }

    #[test]
    fn test_skin_fonts_are_specified() {
        // Test that skin fonts are specified
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        for skin in &skin_manager.available_skins {
            assert!(
                !skin.fonts.proportional.is_empty(),
                "Proportional font should be specified"
            );
            
            assert!(
                !skin.fonts.monospace.is_empty(),
                "Monospace font should be specified"
            );
        }
    }

    #[test]
    fn test_get_active_skin() {
        // Test that get_active_skin returns the correct skin
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        let active_skin = skin_manager.get_active_skin();
        
        assert_eq!(
            active_skin.metadata.name,
            skin_manager.available_skins[skin_manager.active_skin_index].metadata.name,
            "Active skin should match the skin at active_skin_index"
        );
    }

    #[test]
    fn test_skin_count() {
        // Test that we have the expected number of skins
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        // We expect at least 2 skins
        assert!(
            skin_manager.available_skins.len() >= 2,
            "Should have at least 2 skins"
        );
        
        // Print available skins for debugging
        println!("Available skins:");
        for (i, skin) in skin_manager.available_skins.iter().enumerate() {
            println!("  [{}] {}", i, skin.metadata.name);
        }
    }

    #[test]
    fn test_skin_metadata_completeness() {
        // Test that all skins have complete metadata
        let skin_manager = SkinManager::discover_and_load(Path::new("./skins"));
        
        for skin in &skin_manager.available_skins {
            assert!(!skin.metadata.name.is_empty());
            assert!(!skin.metadata.author.is_empty());
            assert!(!skin.metadata.version.is_empty());
            assert!(!skin.metadata.description.is_empty());
        }
    }

    #[test]
    fn test_hex_color_parsing() {
        // Test hex color parsing
        let color = super::super::parser::hex_to_color32("#ffffff");
        assert!(color.is_ok(), "Valid hex color should parse");
        
        let color = super::super::parser::hex_to_color32("#000000");
        assert!(color.is_ok(), "Black hex color should parse");
        
        let color = super::super::parser::hex_to_color32("ffffff");
        assert!(color.is_err(), "Hex color without # should fail");
        
        let color = super::super::parser::hex_to_color32("#gggggg");
        assert!(color.is_err(), "Invalid hex color should fail");
    }

    #[test]
    fn test_skin_default_builtin() {
        // Test that default_builtin creates a valid skin
        let skin = Skin::default_builtin();
        
        assert_eq!(skin.metadata.name, "OneAmp Dark");
        assert!(skin.colors.dark_mode);
        assert!(!skin.colors.background.is_empty());
        assert!(!skin.colors.text.is_empty());
    }

    #[test]
    fn test_colors_default() {
        // Test Colors default implementation
        let colors = Colors::default();
        
        assert!(colors.dark_mode);
        assert_eq!(colors.background, "#0a0a0a");
        assert_eq!(colors.text, "#ffffff");
        assert!(!colors.accent.is_empty());
    }

    #[test]
    fn test_fonts_default() {
        // Test Fonts default implementation
        let fonts = Fonts::default();
        
        assert_eq!(fonts.proportional, "Arial");
        assert_eq!(fonts.monospace, "Courier New");
    }

    #[test]
    fn test_metrics_default() {
        // Test Metrics default implementation
        let metrics = Metrics::default();
        
        assert_eq!(metrics.window_rounding, 4.0);
        assert_eq!(metrics.widget_rounding, 2.0);
        assert_eq!(metrics.timer_text_size, 48.0);
    }
}
