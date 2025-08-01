use jailed_path::{JailedPath, JailedPathError, PathValidator};

// Define marker types for different resource categories
pub struct ImageResource;
pub struct UserData;

fn main() -> Result<(), JailedPathError> {
    let current_dir = std::env::current_dir().unwrap();

    // Unmarked validator (no generics needed)
    let generic_validator: PathValidator = PathValidator::with_jail(&current_dir)?;
    match generic_validator.try_path("Cargo.toml") {
        Ok(generic_path) => println!("Generic path: {}", generic_path.virtual_display()),
        Err(e) => println!("Error: {e}"),
    }

    // Marked validator for images
    let image_validator: PathValidator<ImageResource> = PathValidator::with_jail(&current_dir)?;
    match image_validator.try_path("Cargo.toml") {
        Ok(image_path) => {
            println!("Image resource path: {}", image_path.virtual_display());
            process_images(image_path); // Use the function to avoid dead code warning
        }
        Err(e) => println!("Error: {e}"),
    }

    // Marked validator for user data
    let user_validator: PathValidator<UserData> = PathValidator::with_jail(&current_dir)?;
    match user_validator.try_path("Cargo.toml") {
        Ok(user_path) => {
            println!("User data path: {}", user_path.virtual_display());
            process_user_data(user_path); // Use the function to avoid dead code warning
        }
        Err(e) => println!("Error: {e}"),
    }

    // This would be a compile error if uncommented:
    // process_images(user_path); // Cannot pass UserData to function expecting ImageResource

    println!("Marker types provide compile-time type safety!");
    Ok(())
}

// Function that only accepts image resource paths
fn process_images(_path: JailedPath<ImageResource>) {
    println!("Processing image...");
}

// Function that only accepts user data paths
fn process_user_data(_path: JailedPath<UserData>) {
    println!("Processing user data...");
}
