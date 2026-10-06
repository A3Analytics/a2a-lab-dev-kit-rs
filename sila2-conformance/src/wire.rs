#![allow(clippy::all, dead_code, unused, non_camel_case_types)]

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
                pub mod authenticationservice {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.core.authenticationservice.v1.rs"
                        ));
                    }
                }
                pub mod authorizationservice {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.core.authorizationservice.v1.rs"
                        ));
                    }
                }
            }

            pub mod test {
                pub mod anytypetest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.anytypetest.v1.rs"
                        ));
                    }
                }
                pub mod authenticationtest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.authenticationtest.v1.rs"
                        ));
                    }
                }
                pub mod basicdatatypestest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.basicdatatypestest.v1.rs"
                        ));
                    }
                }
                pub mod binarytransfertest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.binarytransfertest.v1.rs"
                        ));
                    }
                }
                pub mod errorhandlingtest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.errorhandlingtest.v1.rs"
                        ));
                    }
                }
                pub mod listdatatypetest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.listdatatypetest.v1.rs"
                        ));
                    }
                }
                pub mod metadataconsumertest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.metadataconsumertest.v1.rs"
                        ));
                    }
                }
                pub mod metadataprovider {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.metadataprovider.v1.rs"
                        ));
                    }
                }
                pub mod multiclienttest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.multiclienttest.v1.rs"
                        ));
                    }
                }
                pub mod observablecommandtest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.observablecommandtest.v1.rs"
                        ));
                    }
                }
                pub mod observablepropertytest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.observablepropertytest.v1.rs"
                        ));
                    }
                }
                pub mod structuredatatypetest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.structuredatatypetest.v1.rs"
                        ));
                    }
                }
                pub mod unobservablecommandtest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.unobservablecommandtest.v1.rs"
                        ));
                    }
                }
                pub mod unobservablepropertytest {
                    pub mod v1 {
                        include!(concat!(
                            env!("OUT_DIR"),
                            "/sila2.org.silastandard.test.unobservablepropertytest.v1.rs"
                        ));
                    }
                }
            }
        }
    }
}
