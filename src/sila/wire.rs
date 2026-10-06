//! Generated SiLA protobuf modules, included at the package path the stubs expect.

#![allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    dead_code,
    unused,
    non_camel_case_types
)]

pub mod sila2 {
    pub mod org {
        pub mod silastandard {
            include!(concat!(env!("OUT_DIR"), "/sila2.org.silastandard.rs"));

            pub mod core {
                pub mod silaservice {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.core.silaservice.v1.rs"
                        ));
                    }
                }

                pub mod commands {
                    pub mod cancelcontroller {
                        pub mod v1 {
                            include!(concat!(
                                env!("OUT_DIR"),
                                "/sila2.org.silastandard.core.commands.cancelcontroller.v1.rs"
                            ));
                        }
                    }
                }

                pub mod connectionconfigurationservice {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.core.connectionconfigurationservice.v1.rs"
                        ));
                    }
                }
            }
        }
    }

    pub mod com {
        pub mod a3analytics {
            pub mod lab {
                pub mod laboperations {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.com.a3analytics.lab.laboperations.v1.rs"
                        ));
                    }
                }
            }
        }
    }
}
