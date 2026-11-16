//! Home rows for the Store (Steam top sellers) and Game Pass (PC catalogue): title, art and store page.
//! Fetched once in the background; a failed fetch just leaves the row with its "Browse" tile.

#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: String,
    pub name: String,
    pub image: String,
    pub page: String,
}

fn get(url: &str) -> Result<serde_json::Value, String> {
    ureq::get(url).call().map_err(|e| e.to_string())?.body_mut().read_json().map_err(|e| e.to_string())
}

pub fn steam_top_sellers() -> Result<Vec<FeedItem>, String> {
    let v = get("https://store.steampowered.com/api/featuredcategories?cc=us&l=english")?;
    let mut seen = std::collections::HashSet::new();
    Ok(v["top_sellers"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let id = i["id"].as_u64()?;
            let name = i["name"].as_str()?.to_string();
            let image = i["header_image"].as_str().or(i["large_capsule_image"].as_str())?.to_string();
            seen.insert(id).then(|| FeedItem {
                id: id.to_string(),
                name,
                image,
                page: format!("https://store.steampowered.com/app/{id}/"),
            })
        })
        .take(12)
        .collect())
}

/// PC Game Pass "all games" list.
const PC_GAME_PASS: &str = "609d944c-d395-4c0a-9ea4-e9f39b52c1ad";

pub fn game_pass() -> Result<Vec<FeedItem>, String> {
    let list = get(&format!("https://catalog.gamepass.com/sigls/v2?id={PC_GAME_PASS}&language=en-us&market=US"))?;
    let ids: Vec<&str> = list.as_array().into_iter().flatten().filter_map(|i| i["id"].as_str()).take(12).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let products = get(&format!(
        "https://displaycatalog.mp.microsoft.com/v7.0/products?bigIds={}&market=US&languages=en-us",
        ids.join(",")
    ))?;
    Ok(products["Products"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let id = p["ProductId"].as_str()?;
            let props = &p["LocalizedProperties"][0];
            let images = props["Images"].as_array()?;
            let pick =
                |purpose: &str| images.iter().find(|i| i["ImagePurpose"] == purpose).and_then(|i| i["Uri"].as_str());
            let uri = pick("BoxArt").or_else(|| pick("Poster")).or_else(|| pick("Tile"))?;
            Some(FeedItem {
                id: id.to_string(),
                name: props["ProductTitle"].as_str()?.to_string(),
                image: if uri.starts_with("//") { format!("https:{uri}") } else { uri.to_string() },
                page: format!("https://www.xbox.com/en-US/games/store/x/{id}"),
            })
        })
        .collect())
}
