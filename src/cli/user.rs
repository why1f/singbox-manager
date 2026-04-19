use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct AddUserArgs {
    pub name: String,
    #[arg(short,long,default_value="0")]  pub quota:     f64,
    #[arg(short,long,default_value="0")]  pub reset_day: i64,
    #[arg(short,long,default_value="")]   pub expire:    String,
}

#[derive(Args, Debug)]
pub struct PackageArgs {
    pub name: String,
    #[arg(short,long)] pub quota:     Option<f64>,
    #[arg(short,long)] pub reset_day: Option<i64>,
    #[arg(short,long)] pub expire:    Option<String>,
}

#[derive(Args, Debug)]
pub struct UserArgs { #[command(subcommand)] pub command: UserCommands }

#[derive(Subcommand, Debug)]
pub enum UserCommands {
    List,
    Info   { name: String },
    Add    { name: String,
             #[arg(short,long,default_value="0")]  quota:     f64,
             #[arg(short,long,default_value="0")]  reset_day: i64,
             #[arg(short,long,default_value="")]   expire:    String },
    Del    { name: String },
    Toggle { name: String },
    Reset  { name: String },
    Package { name: String,
              #[arg(short,long)] quota:     Option<f64>,
              #[arg(short,long)] reset_day: Option<i64>,
              #[arg(short,long)] expire:    Option<String> },
    AddTraffic { name: String, amount: String },
    Sub    { name: String },
}
