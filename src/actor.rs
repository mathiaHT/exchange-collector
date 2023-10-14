use uuid::Uuid;

/// Actor supertrait for common behaviour
pub trait Actor {
    fn name(&self) -> String;
    fn uuid(&self) -> Uuid;
    fn is_me(&self, uuid: Uuid) -> bool {
        self.uuid() == uuid
    }
}

pub trait ActorMeter {}
