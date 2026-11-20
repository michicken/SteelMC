//! Vanilla `/effect` command support.

use std::str::FromStr;

use steel_registry::{REGISTRY, RegistryExt as _, mob_effect::MobEffectRef};
use steel_utils::Identifier;
use text_components::TextComponent;

use super::super::{
    brigadier::{ArgumentType, CommandNodeBuilder, CommandSyntaxError},
    execution::{
        CommandSource, SteelArgumentType, SteelCommandContext, SteelCommandRuntime, argument,
        literal,
    },
    registration::CommandRegistration,
};
use crate::entity::MobEffectInstance;

pub(super) fn registration() -> CommandRegistration<CommandSource> {
    CommandRegistration::new(Identifier::vanilla_static("effect"), |_| command())
}

fn command() -> CommandNodeBuilder<CommandSource, SteelCommandRuntime> {
    literal("effect")
        .then(literal("give").then(
            argument("targets", SteelArgumentType::entity()).then(
                argument("effect", ArgumentType::string())
                    .executes(give)
                    .then(
                        argument("seconds", ArgumentType::integer(1, 1_000_000))
                            .executes(give)
                            .then(
                                argument("amplifier", ArgumentType::integer(0, 255))
                                    .executes(give)
                                    .then(literal("hideParticles").executes(give)),
                            ),
                    ),
            ),
        ))
        .then(literal("clear").then(
            argument("targets", SteelArgumentType::entity())
                .executes(clear)
                .then(argument("effect", ArgumentType::string()).executes(clear)),
        ))
}

fn parse_effect(context: &SteelCommandContext<CommandSource>) -> Result<MobEffectRef, CommandSyntaxError> {
    let raw = context.string("effect")?;
    let key = Identifier::from_str(raw).map_err(|_| {
        CommandSyntaxError::dynamic(TextComponent::from(format!("Unknown effect: {raw}")))
    })?;
    REGISTRY.mob_effects.by_key(&key).ok_or_else(|| {
        CommandSyntaxError::dynamic(TextComponent::from(format!("Unknown effect: {raw}")))
    })
}

fn give(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    let effect = parse_effect(context)?;
    let seconds = context.integer("seconds").unwrap_or(30);
    let amplifier = context.integer("amplifier").unwrap_or(0);
    let duration = seconds.saturating_mul(20);

    let mut applied = 0;
    for target in targets {
        if let Some(living) = target.as_living_entity()
            && living.add_mob_effect(MobEffectInstance::with_duration(effect, duration, amplifier))
        {
            applied += 1;
        }
    }
    context.source().send_success(
        &TextComponent::from(format!("Applied {effect:?} to {applied} entit(y/ies)")),
        false,
    );
    Ok(applied)
}

fn clear(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    let only_effect = context.string("effect").ok().and_then(|raw| {
        Identifier::from_str(raw)
            .ok()
            .and_then(|key| REGISTRY.mob_effects.by_key(&key))
    });

    let mut cleared = 0;
    for target in targets {
        let Some(living) = target.as_living_entity() else {
            continue;
        };
        if let Some(effect) = only_effect {
            if living.remove_mob_effect(effect) {
                cleared += 1;
            }
        } else {
            let effects = living.active_mob_effects();
            for effect in effects {
                living.remove_mob_effect(effect.effect());
            }
            cleared += 1;
        }
    }
    context.source().send_success(
        &TextComponent::from(format!("Cleared effects from {cleared} entit(y/ies)")),
        false,
    );
    Ok(cleared)
}
