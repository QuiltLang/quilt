pub trait Validate: Sized {
    type Error: Into<miette::Error>;

    fn validate(self) -> Result<Self, Self::Error>;
    fn v(self) -> Result<Self, Self::Error> {
        self.validate()
    }
}
