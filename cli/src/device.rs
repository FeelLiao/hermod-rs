//! 设备身份与联系人命令；所有密钥和文件操作由 core 封装。
use crate::cli::{DeviceArgs, DeviceCommand};
use hermod_core::{config, contacts::Contacts, identity::Identity};
use std::io::{self, Write};

pub fn run(args: DeviceArgs) -> Result<(), Box<dyn std::error::Error>> {
    let dir = config::dir();
    let mut output = io::BufWriter::new(io::stdout().lock());
    match args.command {
        DeviceCommand::Id => {
            let identity = Identity::load_or_create(&dir)?;
            writeln!(output, "{}", identity.device_id())?;
        }
        DeviceCommand::List => {
            let contacts = Contacts::open(&dir)?;
            for contact in contacts.list()? {
                writeln!(
                    output,
                    "{}\t{}\t{:?}",
                    contact.device_id, contact.name, contact.state
                )?;
            }
        }
        DeviceCommand::Rename { id, name } => Contacts::open(&dir)?.rename(id, &name)?,
        DeviceCommand::Remove { id } => Contacts::open(&dir)?.remove(id)?,
    }
    output.flush()?;
    Ok(())
}
