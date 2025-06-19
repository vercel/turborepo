use turborepo_ui::{ColorConfig, BOLD, CYAN};

pub fn print_cli_authorized(user: &str, color_config: &ColorConfig) {
    println!(
        "
{} Turborepo CLI authorized for {}
{}
{}
",
        color_config.rainbow(">>> Success!"),
        user,
        color_config.apply(
            CYAN.apply_to("To connect to your Remote Cache, run the following in any turborepo:")
        ),
        color_config.apply(BOLD.apply_to("  npx turbo link"))
    );
}
