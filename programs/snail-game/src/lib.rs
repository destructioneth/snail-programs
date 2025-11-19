use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, Token2022, TokenAccount, FreezeAccount};
use anchor_spl::token_2022::{self, spl_token_2022::instruction::AuthorityType};

declare_id!("2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT");

#[program]
pub mod snail_game {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        snail_start_stamp: i64,
        snail_end_stamp: i64,
        target_market_cap: u64,
        curve_factor: u64,
        usdc_lp: Pubkey,
        snail_lp: Pubkey,
        snail_mint: Pubkey,
    ) -> Result<()> {
        require!(
            snail_end_stamp > snail_start_stamp,
            SnailError::InvalidTimestamps
        );
        require!(curve_factor <= 100, SnailError::InvalidCurveFactor);
        
        let game_state = &mut ctx.accounts.game_state;
        game_state.owner = ctx.accounts.owner.key();
        game_state.snail_start_stamp = snail_start_stamp;
        game_state.snail_end_stamp = snail_end_stamp;
        game_state.target_market_cap = target_market_cap;
        game_state.curve_factor = curve_factor;
        game_state.usdc_lp = usdc_lp;
        game_state.snail_lp = snail_lp;
        game_state.snail_mint = snail_mint;
        game_state.configured = true;
        
        Ok(())
    }

    /// Check the required market cap at a given timestamp
    pub fn check_required_market_cap(ctx: Context<CheckRequiredMarketCap>, timestamp: i64) -> Result<u64> {
        let game_state = &ctx.accounts.game_state;
        
        // Return 0 if not configured or timestamps are invalid
        if !game_state.configured || game_state.snail_end_stamp == 0 {
            return Ok(0);
        }
        
        // Return 0 if before start or after end
        if timestamp < game_state.snail_start_stamp || timestamp >= game_state.snail_end_stamp {
            return Ok(0);
        }
        
        // Calculate progress (0 to 1, scaled by 1e18 for precision)
        let elapsed = (timestamp - game_state.snail_start_stamp) as u64;
        let duration = (game_state.snail_end_stamp - game_state.snail_start_stamp) as u64;
        let progress = ((elapsed as u128) * 1_000_000_000_000_000_000u128) / (duration as u128);
        
        // Apply curve: progress^(1 + curveFactor * 0.4)
        // curveFactor is stored with 1 decimal, so divide by 10
        // Exponent = 1 + (curveFactor / 10) * 0.4 = 1 + curveFactor * 0.04
        // Scaled: exponent = 1e18 + curveFactor * 4e16
        let exponent = 1_000_000_000_000_000_000u128 + ((game_state.curve_factor as u128) * 400_000_000_000_000_00u128);
        
        // Calculate curved progress
        let curved_progress = pow(progress, exponent)?;
        
        // Calculate required market cap
        let required_market_cap = ((game_state.target_market_cap as u128) * curved_progress) / 1_000_000_000_000_000_000u128;
        
        Ok(required_market_cap as u64)
    }

    /// Check the current market cap
    pub fn check_current_market_cap(ctx: Context<CheckCurrentMarketCap>) -> Result<u64> {
        let game_state = &ctx.accounts.game_state;
        require!(game_state.configured, SnailError::NotConfigured);
        
        let usdc_lp_account = &ctx.accounts.usdc_lp;
        let snail_lp_account = &ctx.accounts.snail_lp;
        let snail_mint_account = &ctx.accounts.snail_mint;
        
        // Get reserves from LP token accounts
        let snail_reserve = snail_lp_account.amount;
        let usdc_reserve = usdc_lp_account.amount;
        
        // Avoid division by zero
        if snail_reserve == 0 {
            return Ok(0);
        }
        
        // Calculate market cap: (usdcReserve * totalSupply) / snailReserve
        let total_supply = snail_mint_account.supply;
        let market_cap = ((usdc_reserve as u128) * (total_supply as u128)) / (snail_reserve as u128);
        
        Ok(market_cap as u64)
    }

    /// Touch the snail - check if market cap is at or below required, and freeze if so
    pub fn touch_snail(ctx: Context<TouchSnail>) -> Result<()> {
        let game_state = &ctx.accounts.game_state;
        let clock = Clock::get()?;
        
        require!(game_state.configured, SnailError::NotConfigured);
        require!(!game_state.frozen, SnailError::AlreadyFrozen);
        
        // Calculate current market cap
        let usdc_lp_account = &ctx.accounts.usdc_lp;
        let snail_lp_account = &ctx.accounts.snail_lp;
        let snail_mint_account = &ctx.accounts.snail_mint;
        
        let snail_reserve = snail_lp_account.amount;
        let usdc_reserve = usdc_lp_account.amount;
        
        require!(snail_reserve > 0, SnailError::InvalidReserves);
        
        let total_supply = snail_mint_account.supply;
        let current_market_cap = ((usdc_reserve as u128) * (total_supply as u128)) / (snail_reserve as u128);
        
        // Calculate required market cap at current time
        let timestamp = clock.unix_timestamp;
        let required_market_cap = if timestamp < game_state.snail_start_stamp || timestamp >= game_state.snail_end_stamp {
            0u128
        } else {
            // Calculate progress (0 to 1, scaled by 1e18 for precision)
            let elapsed = (timestamp - game_state.snail_start_stamp) as u64;
            let duration = (game_state.snail_end_stamp - game_state.snail_start_stamp) as u64;
            let progress = ((elapsed as u128) * 1_000_000_000_000_000_000u128) / (duration as u128);
            
            // Apply curve: progress^(1 + curveFactor * 0.4)
            let exponent = 1_000_000_000_000_000_000u128 + ((game_state.curve_factor as u128) * 400_000_000_000_000_00u128);
            let curved_progress = pow(progress, exponent)?;
            
            // Calculate required market cap
            ((game_state.target_market_cap as u128) * curved_progress) / 1_000_000_000_000_000_000u128
        };
        
        require!(required_market_cap > 0, SnailError::InvalidTimestamps);
        
        // Only proceed if current is at or below required
        require!(
            current_market_cap <= required_market_cap,
            SnailError::MarketCapTooHigh
        );
        
        // Mark as frozen
        let game_state_mut = &mut ctx.accounts.game_state;
        game_state_mut.frozen = true;
        
        // Freeze the snail LP account
        let seeds = &[
            b"freeze-authority".as_ref(),
            &[ctx.bumps.freeze_authority],
        ];
        let signer = &[&seeds[..]];
        
        token_interface::freeze_account(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                FreezeAccount {
                    account: ctx.accounts.snail_lp.to_account_info(),
                    mint: ctx.accounts.snail_mint.to_account_info(),
                    authority: ctx.accounts.freeze_authority.to_account_info(),
                },
                signer,
            ),
        )?;
        
        // Renounce freeze authority (set to None)
        token_2022::set_authority(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::SetAuthority {
                    current_authority: ctx.accounts.freeze_authority.to_account_info(),
                    account_or_mint: ctx.accounts.snail_mint.to_account_info(),
                },
                signer,
            ),
            AuthorityType::FreezeAccount,
            None, // Revoke (set to None)
        )?;
        
        emit!(SnailTouched {
            current_market_cap: current_market_cap as u64,
            required_market_cap: required_market_cap as u64,
        });
        
        Ok(())
    }
}

/// Internal helper function for power calculation (exact copy of Solidity _pow)
fn pow(base: u128, exponent: u128) -> Result<u128> {
    // Handle edge cases
    if base == 0 {
        return Ok(0);
    }
    if exponent == 0 {
        return Ok(1_000_000_000_000_000_000u128);
    }
    if exponent == 1_000_000_000_000_000_000u128 {
        return Ok(base);
    }
    if base == 1_000_000_000_000_000_000u128 {
        return Ok(1_000_000_000_000_000_000u128);
    }
    
    let integer_part = exponent / 1_000_000_000_000_000_000u128;
    let fractional_part = exponent % 1_000_000_000_000_000_000u128;
    
    // Start with base^integerPart
    let mut result = 1_000_000_000_000_000_000u128;
    for _ in 0..integer_part {
        result = result
            .checked_mul(base)
            .ok_or(SnailError::MathOverflow)?
            / 1_000_000_000_000_000_000u128;
    }
    
    // For fractional part, use linear interpolation between base^n and base^(n+1)
    if fractional_part > 0 {
        let next_power = result
            .checked_mul(base)
            .ok_or(SnailError::MathOverflow)?
            / 1_000_000_000_000_000_000u128;
        let diff = result
            .checked_sub(next_power)
            .ok_or(SnailError::MathOverflow)?;
        result = result
            .checked_sub((diff.checked_mul(fractional_part).ok_or(SnailError::MathOverflow)?) / 1_000_000_000_000_000_000u128)
            .ok_or(SnailError::MathOverflow)?;
    }
    
    Ok(result)
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = owner,
        space = 8 + GameState::LEN,
        seeds = [b"game_state"],
        bump
    )]
    pub game_state: Account<'info, GameState>,
    
    #[account(mut)]
    pub owner: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CheckRequiredMarketCap<'info> {
    #[account(
        seeds = [b"game_state"],
        bump
    )]
    pub game_state: Account<'info, GameState>,
}

#[derive(Accounts)]
pub struct CheckCurrentMarketCap<'info> {
    #[account(
        seeds = [b"game_state"],
        bump
    )]
    pub game_state: Account<'info, GameState>,
    
    /// CHECK: USDC LP token account
    pub usdc_lp: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: SNAIL LP token account
    pub snail_lp: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: SNAIL mint account
    pub snail_mint: InterfaceAccount<'info, Mint>,
    
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct TouchSnail<'info> {
    #[account(
        mut,
        seeds = [b"game_state"],
        bump
    )]
    pub game_state: Account<'info, GameState>,
    
    /// CHECK: USDC LP token account
    pub usdc_lp: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: SNAIL LP token account (will be frozen)
    #[account(mut)]
    pub snail_lp: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: SNAIL mint account
    #[account(mut)]
    pub snail_mint: InterfaceAccount<'info, Mint>,
    
    /// CHECK: Freeze authority PDA (will be renounced)
    #[account(
        seeds = [b"freeze-authority"],
        bump,
    )]
    pub freeze_authority: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token2022>,
}


#[account]
pub struct GameState {
    pub owner: Pubkey,
    pub snail_start_stamp: i64,
    pub snail_end_stamp: i64,
    pub target_market_cap: u64,
    pub curve_factor: u64, // Stored with 1 decimal precision (77 = 7.7)
    pub usdc_lp: Pubkey,
    pub snail_lp: Pubkey,
    pub snail_mint: Pubkey,
    pub configured: bool,
    pub frozen: bool,
}

impl GameState {
    pub const LEN: usize = 8 + // discriminator
        32 + // owner
        8 + // snail_start_stamp
        8 + // snail_end_stamp
        8 + // target_market_cap
        8 + // curve_factor
        32 + // usdc_lp
        32 + // snail_lp
        32 + // snail_mint
        1 + // configured
        1; // frozen
}

#[error_code]
pub enum SnailError {
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Invalid timestamps")]
    InvalidTimestamps,
    #[msg("Invalid curve factor")]
    InvalidCurveFactor,
    #[msg("Not configured")]
    NotConfigured,
    #[msg("Already frozen")]
    AlreadyFrozen,
    #[msg("Market cap too high")]
    MarketCapTooHigh,
    #[msg("Invalid reserves")]
    InvalidReserves,
    #[msg("Math overflow")]
    MathOverflow,
}

#[event]
pub struct SnailTouched {
    pub current_market_cap: u64,
    pub required_market_cap: u64,
}
