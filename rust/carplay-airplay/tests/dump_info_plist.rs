// [local PoC patch — not upstream] 诊断用：把我们实际发给 iPhone 的 AirPlay info plist 紧凑打出来，
// 便于和已知能跑顺的实现逐字段对比（重点：features 掩码、display 的 maxFPS/features/viewAreas）。
// 用法：cargo test --release -p carplay-airplay --test dump_info_plist -- --nocapture
use carplay_airplay::air_play_config::AirPlayConfig;
use carplay_airplay::air_play_info_plist::AirPlayInfoPlist;
use carplay_airplay::bplist_codec::BplistValue as V;

fn get<'a>(d: &'a V, key: &str) -> Option<&'a V> {
    match d {
        V::Dictionary(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn keys(d: &V) -> Vec<String> {
    match d {
        V::Dictionary(pairs) => pairs.iter().map(|(k, _)| k.clone()).collect(),
        _ => vec![],
    }
}

fn scalar(v: &V) -> String {
    match v {
        V::Integer(i) => i.to_string(),
        V::Boolean(b) => b.to_string(),
        V::String(s) => format!("{s:?}"),
        V::Real(r) => r.to_string(),
        V::Array(a) => format!("<array len={}>", a.len()),
        V::Dictionary(p) => {
            format!("<dict keys={:?}>", p.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>())
        }
        other => format!("{other:?}"),
    }
}

#[test]
fn dump_our_info_plist() {
    let mut config =
        AirPlayConfig::linux_receiver("AA:BB:CC:DD:EE:FF".into(), "AA:BB:CC:DD:EE:FF".into());
    config.disable_audio_output = true;
    config.output_opus = false;
    config.microphone_opus = false;
    config.main.width_pixels = 1280;
    config.main.height_pixels = 720;
    config.main.width_physical_mm = Some(170);
    config.main.fps = 60;

    let plist = AirPlayInfoPlist::build(&config).expect("plist");
    println!("=== OUR INFO PLIST (compact) ===");
    println!("top-level keys: {:?}", keys(&plist));
    for k in keys(&plist) {
        if let Some(v) = get(&plist, &k) {
            println!("  {k} = {}", scalar(v));
        }
    }
    if let Some(V::Array(displays)) = get(&plist, "displays") {
        for (i, d) in displays.iter().enumerate() {
            println!("displays[{i}] keys: {:?}", keys(d));
            for k in [
                "type",
                "uuid",
                "maxFPS",
                "widthPixels",
                "heightPixels",
                "widthPhysical",
                "heightPhysical",
                "features",
                "primaryInputDevice",
                "uiAppearanceMode",
                "uiAppearanceSetting",
                "mapAppearanceMode",
                "mapAppearanceSetting",
            ] {
                if let Some(v) = get(d, k) {
                    println!("  {k} = {}", scalar(v));
                }
            }
            if let Some(V::Array(areas)) = get(d, "viewAreas") {
                println!("  viewAreas len={}", areas.len());
                if let Some(first) = areas.first() {
                    println!("  viewArea[0] keys: {:?}", keys(first));
                    for k in keys(first) {
                        if let Some(v) = get(first, &k) {
                            println!("    {k} = {}", scalar(v));
                        }
                    }
                }
            }
        }
    }
    if let Some(modes) = get(&plist, "modes") {
        println!("modes keys: {:?}", keys(modes));
        for k in keys(modes) {
            if let Some(v) = get(modes, &k) {
                println!("  modes.{k} = {}", scalar(v));
            }
        }
    }
}
