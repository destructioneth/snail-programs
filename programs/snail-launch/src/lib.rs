use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_2022::{self, spl_token_2022::instruction::AuthorityType};
use anchor_spl::token_interface::{self, Mint, MintTo, Token2022, TokenAccount, TransferChecked};

declare_id!("8ondokpt7wa5mWsr4wSEZe7N3YtkLoPNRy39ovydwyXt");

#[program]
pub mod snail_launch {
    use super::*;

    // ============================================================================
    // INITIALIZATION
    // ============================================================================

    /// Initialize the launch program and mint full supply to treasury
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let launch_state = &mut ctx.accounts.launch_state;
        launch_state.owner = ctx.accounts.owner.key();
        launch_state.snail_mint = ctx.accounts.snail_mint.key();
        launch_state.initialized = true;
        
        // Initialize all distribution states
        launch_state.admin_claimed = false;
        launch_state.sale_configured = false;
        
        // Constants: MAX_SUPPLY = 1,000,000 tokens with 9 decimals
        // Calculate directly without error handling (const context)
        const MAX_SUPPLY: u64 = 1_000_000_000_000_000u64; // 1M * 10^9
        
        // Mint full supply to treasury token account
        // Derive mint authority PDA bump manually
        let (mint_authority_pda, mint_authority_bump) = Pubkey::find_program_address(
            &[b"mint_authority"],
            ctx.program_id
        );
        require!(
            mint_authority_pda == ctx.accounts.mint_authority.key(),
            LaunchError::InvalidMintAuthority
        );
        let seeds = &[
            b"mint_authority".as_ref(),
            &[mint_authority_bump]
        ];
        let signer = &[&seeds[..]];
        
        token_interface::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.snail_mint.to_account_info(),
                    to: ctx.accounts.treasury_token_account.to_account_info(),
                    authority: ctx.accounts.mint_authority.to_account_info(),
                },
                signer,
            ),
            MAX_SUPPLY,
        )?;
        
        // Revoke mint authority so no more tokens can be minted
        token_2022::set_authority(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::SetAuthority {
                    current_authority: ctx.accounts.mint_authority.to_account_info(),
                    account_or_mint: ctx.accounts.snail_mint.to_account_info(),
                },
                signer,
            ),
            AuthorityType::MintTokens,
            None, // Revoke (set to None)
        )?;
        
        emit!(Initialized {
            owner: ctx.accounts.owner.key(),
            snail_mint: ctx.accounts.snail_mint.key(),
            total_supply: MAX_SUPPLY,
        });
        
        Ok(())
    }

    // ============================================================================
    // ADMIN/LP CLAIM (20% = 200k tokens)
    // ============================================================================

    /// Admin can claim the admin/LP portion (20% of total supply)
    /// Claim admin LP tokens (200k tokens)
    /// ATA must be created by the frontend before calling this function
    pub fn claim_admin_lp(ctx: Context<ClaimAdminLp>) -> Result<()> {
        let launch_state = &mut ctx.accounts.launch_state;
        
        require!(
            ctx.accounts.owner.key() == launch_state.owner,
            LaunchError::Unauthorized
        );
        require!(!launch_state.admin_claimed, LaunchError::AdminAlreadyClaimed);
        require!(
            ctx.accounts.snail_mint.key() == launch_state.snail_mint,
            LaunchError::InvalidMint
        );
        
        launch_state.admin_claimed = true;
        
        let admin_lp_supply = 200_000u64
            .checked_mul(10u64.pow(ctx.accounts.snail_mint.decimals as u32))
            .ok_or(LaunchError::MathOverflow)?;
        
        let (treasury_pda, treasury_bump) = Pubkey::find_program_address(
            &[b"treasury"],
            ctx.program_id
        );
        require!(
            treasury_pda == ctx.accounts.treasury_pda.key(),
            LaunchError::InvalidTreasury
        );
        let seeds = &[
            b"treasury".as_ref(),
            &[treasury_bump]
        ];
        let signer = &[&seeds[..]];
        
        token_2022::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.treasury_token_account.to_account_info(),
                    to: ctx.accounts.admin_token_account.to_account_info(),
                    authority: ctx.accounts.treasury_pda.to_account_info(),
                    mint: ctx.accounts.snail_mint.to_account_info(),
                },
                signer,
            ),
            admin_lp_supply,
            ctx.accounts.snail_mint.decimals,
        )?;
        
        emit!(AdminLPClaimed {
            owner: ctx.accounts.owner.key(),
            snail_amount: admin_lp_supply,
        });
        
        Ok(())
    }

    // ============================================================================
    // PUBLIC SALE (40% = 400k tokens)
    // ============================================================================

    /// Initialize the public sale
    pub fn initialize_sale(
        ctx: Context<InitializeSale>,
        start_time: i64,
        end_time: i64,
        claim_stamp: i64, // Timestamp when claiming becomes available (after sale ends)
    ) -> Result<()> {
        require!(end_time > start_time, LaunchError::InvalidTimestamps);
        require!(claim_stamp >= end_time, LaunchError::InvalidClaimStamp);
        
        let launch_state = &mut ctx.accounts.launch_state;
        
        require!(
            ctx.accounts.owner.key() == launch_state.owner,
            LaunchError::Unauthorized
        );
        
        launch_state.sale_start_time = start_time;
        launch_state.sale_end_time = end_time;
        launch_state.claim_stamp = claim_stamp;
        // Don't reset total_sol_raised - it should persist across sale reconfigurations
        launch_state.sale_admin_claimed = false;
        launch_state.sale_configured = true;
        
        emit!(PublicSaleConfigured {
            start_time,
            end_time,
            claim_stamp,
        });
        
        Ok(())
    }

    /// Contribute SOL to the public sale
    pub fn contribute(ctx: Context<Contribute>, amount: u64) -> Result<()> {
        let launch_state = &mut ctx.accounts.launch_state;
        let clock = Clock::get()?;
        
        require!(
            clock.unix_timestamp >= launch_state.sale_start_time &&
            clock.unix_timestamp <= launch_state.sale_end_time,
            LaunchError::SaleNotActive
        );
        
        // Transfer SOL from contributor to sale vault using SystemProgram::transfer
        anchor_lang::solana_program::program::invoke(
            &anchor_lang::solana_program::system_instruction::transfer(
                ctx.accounts.contributor.key,
                ctx.accounts.sale_vault.key,
                amount,
            ),
            &[
                ctx.accounts.contributor.to_account_info(),
                ctx.accounts.sale_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        
        // Track contribution
        let contributor_data = &mut ctx.accounts.contributor_data;
        contributor_data.amount = contributor_data.amount
            .checked_add(amount)
            .ok_or(LaunchError::MathOverflow)?;
        
        launch_state.total_sol_raised = launch_state.total_sol_raised
            .checked_add(amount)
            .ok_or(LaunchError::MathOverflow)?;
        
        emit!(ContributionReceived {
            contributor: ctx.accounts.contributor.key(),
            amount,
        });
        
        Ok(())
    }

    /// Claim SNAIL tokens based on SOL contribution
    /// Can only be called after claim_stamp timestamp
    pub fn claim_snail(ctx: Context<ClaimSnail>) -> Result<()> {
        let launch_state = &ctx.accounts.launch_state;
        let clock = Clock::get()?;
        
        require!(
            clock.unix_timestamp >= launch_state.claim_stamp,
            LaunchError::ClaimNotAvailable
        );
        
        let contributor_data = &mut ctx.accounts.contributor_data;
        
        require!(contributor_data.amount > 0, LaunchError::NoContribution);
        require!(!contributor_data.claimed, LaunchError::AlreadyClaimed);
        
        let public_sale_supply = 400_000u64
            .checked_mul(10u64.pow(ctx.accounts.snail_mint.decimals as u32))
            .ok_or(LaunchError::MathOverflow)?;
        
        let snail_amount = (contributor_data.amount as u128)
            .checked_mul(public_sale_supply as u128)
            .ok_or(LaunchError::MathOverflow)?
            .checked_div(launch_state.total_sol_raised as u128)
            .ok_or(LaunchError::MathOverflow)?;
        
        contributor_data.claimed = true;
        
        let (treasury_pda, treasury_bump) = Pubkey::find_program_address(
            &[b"treasury"],
            ctx.program_id
        );
        require!(
            treasury_pda == ctx.accounts.treasury_pda.key(),
            LaunchError::InvalidTreasury
        );
        require!(
            ctx.accounts.snail_mint.key() == launch_state.snail_mint,
            LaunchError::InvalidMint
        );
        let seeds = &[
            b"treasury".as_ref(),
            &[treasury_bump]
        ];
        let signer = &[&seeds[..]];
        
        token_2022::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.treasury_token_account.to_account_info(),
                    to: ctx.accounts.contributor_token_account.to_account_info(),
                    authority: ctx.accounts.treasury_pda.to_account_info(),
                    mint: ctx.accounts.snail_mint.to_account_info(),
                },
                signer,
            ),
            snail_amount as u64,
            ctx.accounts.snail_mint.decimals,
        )?;
        
        emit!(SnailClaimed {
            claimer: ctx.accounts.contributor.key(),
            snail_amount: snail_amount as u64,
        });
        
        Ok(())
    }

    /// View function to check available SNAIL tokens for an address
    pub fn snail_available(ctx: Context<SnailAvailable>) -> Result<u64> {
        let launch_state = &ctx.accounts.launch_state;
        let contributor_data = &ctx.accounts.contributor_data;
        
        if contributor_data.amount == 0 || contributor_data.claimed {
            return Ok(0);
        }
        
        if launch_state.total_sol_raised == 0 {
            return Ok(0);
        }
        
        let public_sale_supply = 400_000u64
            .checked_mul(10u64.pow(ctx.accounts.snail_mint.decimals as u32))
            .ok_or(LaunchError::MathOverflow)?;
        
        let snail_amount = (contributor_data.amount as u128)
            .checked_mul(public_sale_supply as u128)
            .ok_or(LaunchError::MathOverflow)?
            .checked_div(launch_state.total_sol_raised as u128)
            .ok_or(LaunchError::MathOverflow)?;
        
        Ok(snail_amount as u64)
    }

    /// Admin can claim all SOL after sale ends
    pub fn claim_admin_sol(ctx: Context<ClaimAdminSol>) -> Result<()> {
        let launch_state = &mut ctx.accounts.launch_state;
        let clock = Clock::get()?;
        
        require!(
            clock.unix_timestamp > launch_state.sale_end_time,
            LaunchError::SaleNotEnded
        );
        require!(
            ctx.accounts.owner.key() == launch_state.owner,
            LaunchError::Unauthorized
        );
        require!(!launch_state.sale_admin_claimed, LaunchError::AdminAlreadyClaimed);
        
        // Derive sale vault PDA and verify
        let (sale_vault_pda, sale_vault_bump) = Pubkey::find_program_address(
            &[b"sale_vault"],
            ctx.program_id
        );
        require!(
            sale_vault_pda == ctx.accounts.sale_vault.key(),
            LaunchError::InvalidTreasury
        );
        
        launch_state.sale_admin_claimed = true;
        
        // Transfer SOL from sale vault PDA to admin using SystemProgram::transfer
        // The sale_vault PDA needs to sign this transaction
        let vault_lamports = ctx.accounts.sale_vault.lamports();
        
        // Get minimum rent for a system account (PDA with no data)
        let rent = anchor_lang::solana_program::rent::Rent::get()?;
        let min_rent = rent.minimum_balance(0); // 0 bytes of data for a simple system account
        
        // Calculate transferable amount (all lamports minus rent-exempt reserve)
        let transferable_lamports = vault_lamports
            .checked_sub(min_rent)
            .ok_or(LaunchError::MathOverflow)?;
        
        // Use system_program::transfer with PDA as signer
        let seeds = &[
            b"sale_vault".as_ref(),
            &[sale_vault_bump]
        ];
        let signer = &[&seeds[..]];
        
        anchor_lang::solana_program::program::invoke_signed(
            &anchor_lang::solana_program::system_instruction::transfer(
                ctx.accounts.sale_vault.key,
                ctx.accounts.owner.key,
                transferable_lamports,
            ),
            &[
                ctx.accounts.sale_vault.to_account_info(),
                ctx.accounts.owner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer,
        )?;
        
        emit!(AdminSolClaimed {
            owner: ctx.accounts.owner.key(),
            sol_amount: transferable_lamports,
        });
        
        Ok(())
    }

    // ============================================================================
    // AIRDROP (40% = 400k tokens)
    // ============================================================================

    /// Admin sends tokens to a single ATA
    /// ATA must be created by the frontend before calling this function
    pub fn airdrop(
        ctx: Context<Airdrop>,
        amount: u64,
    ) -> Result<()> {
        let launch_state = &ctx.accounts.launch_state;

        require!(
            ctx.accounts.owner.key() == launch_state.owner,
            LaunchError::Unauthorized
        );

        require!(
            ctx.accounts.snail_mint.key() == launch_state.snail_mint,
            LaunchError::InvalidMint
        );
        
        let (treasury_pda, treasury_bump) = Pubkey::find_program_address(
            &[b"treasury"],
            ctx.program_id
        );
        require!(
            treasury_pda == ctx.accounts.treasury_pda.key(),
            LaunchError::InvalidTreasury
        );
        
        let seeds = &[
            b"treasury".as_ref(),
            &[treasury_bump]
        ];
        let signer = &[&seeds[..]];
        
        // Transfer tokens from treasury to recipient ATA
        token_2022::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.treasury_token_account.to_account_info(),
                    to: ctx.accounts.recipient_token_account.to_account_info(),
                    authority: ctx.accounts.treasury_pda.to_account_info(),
                    mint: ctx.accounts.snail_mint.to_account_info(),
                },
                signer,
            ),
            amount,
            ctx.accounts.snail_mint.decimals,
        )?;
        
        emit!(AirdropSent {
            recipient: ctx.accounts.recipient_token_account.key(),
            amount,
        });
        
        Ok(())
    }

    /// Revoke ownership of the contract, setting owner to System Program
    pub fn revoke_ownership(ctx: Context<RevokeOwnership>) -> Result<()> {
        let launch_state = &mut ctx.accounts.launch_state;
        
        require!(
            ctx.accounts.owner.key() == launch_state.owner,
            LaunchError::Unauthorized
        );
        
        // Set owner to System Program (all zeros)
        launch_state.owner = Pubkey::default();
        
        emit!(OwnershipRevoked {
            previous_owner: ctx.accounts.owner.key(),
        });
        
        Ok(())
    }
}

// ============================================================================
// ACCOUNT STRUCTS
// ============================================================================

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = owner,
        space = 8 + LaunchState::LEN,
        seeds = [b"launch_state"],
        bump
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    #[account(mut)]
    pub owner: Signer<'info>,
    
    /// Snail mint account (Token-2022)
    pub snail_mint: InterfaceAccount<'info, Mint>,
    
    /// CHECK: Treasury PDA (authority for treasury token account)
    #[account(
        seeds = [b"treasury"],
        bump
    )]
    pub treasury_pda: AccountInfo<'info>,
    
    /// Treasury token account (ATA) - holds all minted tokens
    /// Authority is treasury_pda (program signs with treasury seeds)
    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = snail_mint,
        associated_token::authority = treasury_pda,
        token::token_program = token_program
    )]
    pub treasury_token_account: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: Mint authority PDA (will be revoked after minting)
    #[account(
        seeds = [b"mint_authority"],
        bump
    )]
    pub mint_authority: AccountInfo<'info>,
    
    /// Token program (Token-2022)
    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimAdminLp<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump,
        has_one = owner @ LaunchError::Unauthorized
    )]
    pub launch_state: Account<'info, LaunchState>,

    #[account(mut)]
    pub owner: Signer<'info>,

    /// Snail mint account (Token-2022)
    pub snail_mint: InterfaceAccount<'info, Mint>,

    /// CHECK: Admin's token account (ATA) - must be created by frontend before calling this function
    #[account(mut)]
    pub admin_token_account: UncheckedAccount<'info>,

    /// CHECK: Treasury PDA (authority for treasury token account)
    #[account(
        seeds = [b"treasury"],
        bump
    )]
    pub treasury_pda: AccountInfo<'info>,

    /// CHECK: Treasury token account - holds all tokens
    #[account(mut)]
    pub treasury_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct InitializeSale<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump,
        has_one = owner @ LaunchError::Unauthorized
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct Contribute<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    #[account(mut)]
    pub contributor: Signer<'info>,
    
    #[account(
        init_if_needed,
        payer = contributor,
        space = 8 + ContributorData::LEN,
        seeds = [b"contributor", contributor.key().as_ref()],
        bump
    )]
    pub contributor_data: Account<'info, ContributorData>,
    
    /// CHECK: Sale vault for SOL
    #[account(
        mut,
        seeds = [b"sale_vault"],
        bump
    )]
    pub sale_vault: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimSnail<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    /// CHECK: Signer is validated by Anchor's Signer type
    #[account(mut)]
    pub contributor: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"contributor", contributor.key().as_ref()],
        bump
    )]
    pub contributor_data: Account<'info, ContributorData>,
    
    /// Snail mint account (Token-2022)
    pub snail_mint: InterfaceAccount<'info, Mint>,
    
    /// CHECK: Contributor's token account (ATA) - must be created by frontend
    #[account(mut)]
    pub contributor_token_account: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: Treasury PDA (authority for treasury token account)
    #[account(
        seeds = [b"treasury"],
        bump
    )]
    pub treasury_pda: AccountInfo<'info>,
    
    /// CHECK: Treasury token account - holds all tokens
    #[account(mut)]
    pub treasury_token_account: InterfaceAccount<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct SnailAvailable<'info> {
    #[account(
        seeds = [b"launch_state"],
        bump
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    #[account(
        seeds = [b"contributor", contributor.key().as_ref()],
        bump
    )]
    pub contributor_data: Account<'info, ContributorData>,
    
    /// CHECK: Contributor address is validated by the contributor_data PDA derivation
    pub contributor: AccountInfo<'info>,
    
    /// Snail mint account (Token-2022)
    pub snail_mint: InterfaceAccount<'info, Mint>,
    
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct ClaimAdminSol<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump,
        has_one = owner @ LaunchError::Unauthorized
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    /// CHECK: Sale vault PDA for SOL storage
    #[account(
        mut,
        seeds = [b"sale_vault"],
        bump
    )]
    pub sale_vault: AccountInfo<'info>,
    
    #[account(mut)]
    pub owner: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Airdrop<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump,
        has_one = owner @ LaunchError::Unauthorized
    )]
    pub launch_state: Account<'info, LaunchState>,

    #[account(mut)]
    pub owner: Signer<'info>,

    /// Snail mint account (Token-2022)
    pub snail_mint: InterfaceAccount<'info, Mint>,
    
    /// CHECK: Recipient's token account (ATA) - must be created by frontend
    #[account(mut)]
    pub recipient_token_account: InterfaceAccount<'info, TokenAccount>,
    
    /// CHECK: Treasury PDA (authority for treasury token account)
    #[account(
        seeds = [b"treasury"],
        bump
    )]
    pub treasury_pda: AccountInfo<'info>,
    
    /// CHECK: Treasury token account - holds all tokens
    #[account(mut)]
    pub treasury_token_account: InterfaceAccount<'info, TokenAccount>,
    
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct RevokeOwnership<'info> {
    #[account(
        mut,
        seeds = [b"launch_state"],
        bump
    )]
    pub launch_state: Account<'info, LaunchState>,
    
    pub owner: Signer<'info>,
}

// ============================================================================
// STATE STRUCTS
// ============================================================================

#[account]
pub struct LaunchState {
    // Owner and initialization
    pub owner: Pubkey,
    pub snail_mint: Pubkey, // Mint address (stored at initialization)
    pub initialized: bool,
    
    // Admin/LP claim (20%)
    pub admin_claimed: bool,
    
    // Public sale (40%)
    pub sale_configured: bool,
    pub sale_start_time: i64,
    pub sale_end_time: i64,
    pub claim_stamp: i64,    // Universal claim timestamp for both sale and airdrop
    pub total_sol_raised: u64,
    pub sale_admin_claimed: bool,
    
}

impl LaunchState {
    pub const LEN: usize = 8 + // discriminator
        32 + // owner
        32 + // snail_mint
        1 + // initialized
        1 + // admin_claimed
        1 + // sale_configured
        8 + // sale_start_time
        8 + // sale_end_time
        8 + // claim_stamp
        8 + // total_sol_raised
        1; // sale_admin_claimed
}

#[account]
pub struct ContributorData {
    pub amount: u64, // SOL contributed
    pub claimed: bool,
}

impl ContributorData {
    pub const LEN: usize = 8 + // discriminator
        8 + // amount
        1; // claimed
}


// ============================================================================
// ERRORS
// ============================================================================

#[error_code]
pub enum LaunchError {
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Admin already claimed")]
    AdminAlreadyClaimed,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Sale not active")]
    SaleNotActive,
    #[msg("Sale not ended")]
    SaleNotEnded,
    #[msg("Claim not available yet")]
    ClaimNotAvailable,
    #[msg("Invalid mint authority")]
    InvalidMintAuthority,
    #[msg("Invalid mint")]
    InvalidMint,
    #[msg("Invalid treasury")]
    InvalidTreasury,
    #[msg("No contribution")]
    NoContribution,
    #[msg("Already claimed")]
    AlreadyClaimed,
    #[msg("Invalid timestamps")]
    InvalidTimestamps,
    #[msg("Invalid claim stamp")]
    InvalidClaimStamp,
}

// ============================================================================
// EVENTS
// ============================================================================

#[event]
pub struct AdminLPClaimed {
    pub owner: Pubkey,
    pub snail_amount: u64,
}

#[event]
pub struct PublicSaleConfigured {
    pub start_time: i64,
    pub end_time: i64,
    pub claim_stamp: i64,
}

#[event]
pub struct ContributionReceived {
    pub contributor: Pubkey,
    pub amount: u64,
}

#[event]
pub struct SnailClaimed {
    pub claimer: Pubkey,
    pub snail_amount: u64,
}

#[event]
pub struct AdminSolClaimed {
    pub owner: Pubkey,
    pub sol_amount: u64,
}

#[event]
pub struct AirdropSent {
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct OwnershipRevoked {
    pub previous_owner: Pubkey,
}

#[event]
pub struct Initialized {
    pub owner: Pubkey,
    pub snail_mint: Pubkey,
    pub total_supply: u64,
}

