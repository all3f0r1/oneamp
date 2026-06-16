use oneamp_core::wsz::{SkinComponent, WszLoader};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <path-to-skin.wsz>", args[0]);
        eprintln!("Example: {} winamp_classic.wsz", args[0]);
        std::process::exit(1);
    }

    let skin_path = &args[1];

    println!("Loading Winamp skin from: {}", skin_path);
    println!("─────────────────────────────────────────\n");

    match WszLoader::load_from_file(skin_path) {
        Ok(skin) => {
            display_skin_info(&skin);
            display_components(&skin);
            display_regions(&skin);
        }
        Err(e) => {
            eprintln!("Error loading skin: {}", e);
            std::process::exit(1);
        }
    }
}

fn display_skin_info(skin: &oneamp_core::wsz::skin::WszSkin) {
    println!("📦 SKIN INFORMATION");
    println!("─────────────────────────────────────────");
    println!("Name:    {}", skin.metadata.name);

    if let Some(author) = &skin.metadata.author {
        println!("Author:  {}", author);
    }

    if let Some(version) = &skin.metadata.version {
        println!("Version: {}", version);
    }

    println!();
}

fn display_components(skin: &oneamp_core::wsz::skin::WszSkin) {
    println!("🎨 BITMAP COMPONENTS");
    println!("─────────────────────────────────────────");

    let components = [
        (SkinComponent::Main, "Main Window", "275×116"),
        (SkinComponent::CButtons, "Control Buttons", "Varied"),
        (SkinComponent::MonoSter, "VU-Meter", "Varied"),
        (SkinComponent::Numbers, "Digital Numbers", "9×13 each"),
        (SkinComponent::PlayPaus, "Play/Pause Indicator", "Varied"),
        (SkinComponent::PosBar, "Position Bar", "248×10"),
        (SkinComponent::TitleBar, "Title Bar", "275×14"),
        (SkinComponent::Volume, "Volume Slider", "68×13"),
        (SkinComponent::Balance, "Balance Slider", "38×13"),
        (SkinComponent::Pledit, "Playlist Editor", "Varied"),
        (SkinComponent::EqMain, "Equalizer", "275×116"),
    ];

    let mut found_count = 0;

    for (component, name, expected_size) in components.iter() {
        if let Some(bitmap) = skin.get_bitmap(component) {
            println!(
                "✓ {:20} {}×{} (expected: {})",
                name, bitmap.width, bitmap.height, expected_size
            );
            found_count += 1;

            if bitmap.width == 0 || bitmap.height == 0 {
                println!("  ⚠️  Warning: Zero-sized bitmap!");
            }
        } else {
            println!("✗ {:20} (missing)", name);
        }
    }

    let custom_count = skin.bitmaps.len() - found_count;
    if custom_count > 0 {
        println!("\n📁 Additional custom components: {}", custom_count);
    }

    println!("\nTotal components: {}\n", skin.bitmaps.len());
}

fn display_regions(skin: &oneamp_core::wsz::skin::WszSkin) {
    if skin.regions.is_empty() {
        return;
    }

    println!("🔷 CUSTOM REGIONS");
    println!("─────────────────────────────────────────");

    for (i, region) in skin.regions.iter().enumerate() {
        let total_points: usize = region.polygons.iter().map(|p| p.points.len()).sum();
        println!(
            "{}. {} ({} polygon{}, {} points total)",
            i + 1,
            region.name,
            region.polygons.len(),
            if region.polygons.len() == 1 { "" } else { "s" },
            total_points,
        );

        let all_points: Vec<_> = region.polygons.iter().flat_map(|p| &p.points).collect();
        if all_points.len() >= 2 {
            let min_x = all_points.iter().map(|p| p.x).min().unwrap_or(0);
            let max_x = all_points.iter().map(|p| p.x).max().unwrap_or(0);
            let min_y = all_points.iter().map(|p| p.y).min().unwrap_or(0);
            let max_y = all_points.iter().map(|p| p.y).max().unwrap_or(0);

            println!(
                "   Bounds: ({}, {}) to ({}, {})",
                min_x, min_y, max_x, max_y
            );

            let center_x = (min_x + max_x) / 2;
            let center_y = (min_y + max_y) / 2;

            if region.contains(center_x, center_y) {
                println!(
                    "   ✓ Center point ({}, {}) is inside region",
                    center_x, center_y
                );
            }
        }
    }

    println!("\nTotal regions: {}\n", skin.regions.len());
}
