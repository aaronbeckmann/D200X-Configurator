//! Extract Minecraft item textures into assets/minecraft/item.
//! Usage: import-minecraft [client.jar] [out-dir]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let jar = args.get(1).map(std::path::PathBuf::from).or_else(deck_icons::find_minecraft_jar).expect("no Minecraft client jar found; pass its path");
    let out = args.get(2).map(std::path::PathBuf::from).unwrap_or_else(deck_icons::minecraft_default_dir);
    match deck_icons::import_minecraft(&jar, &out) {
        Ok(n) => println!("{n} item icons from {} -> {}", jar.display(), out.display()),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}
