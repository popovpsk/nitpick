mod windows;
pub(crate) use windows::{read, write};

const SERVICE: &str = "nitpick";
const ACCOUNT: &str = "data-key";
