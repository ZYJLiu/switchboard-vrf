use anchor_lang::prelude::*;
use anchor_lang::solana_program::{clock, native_token::LAMPORTS_PER_SOL};
use anchor_lang::system_program;
pub use anchor_spl::token::{Token, TokenAccount};
pub use switchboard_v2::{
    OracleQueueAccountData, PermissionAccountData, SbState, VrfAccountData, VrfRequestRandomness,
};

declare_id!("FXWi8jVNNcyCARo6JckMFPiqzcMhPo585NirdPvD2hva");

const GAME_SEED: &[u8] = b"GAME";
const VAULT_SEED: &[u8] = b"VAULT";
const AMOUNT: u64 = LAMPORTS_PER_SOL / 1000;
const MAX_RESULT: u64 = 2;

#[program]
pub mod vrf {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let mut game_state = ctx.accounts.game_state.load_init()?;
        *game_state = GameState::default();
        game_state.bump = ctx.bumps.get("game_state").unwrap().clone();
        game_state.vrf = ctx.accounts.vrf.key();
        game_state.max_result = MAX_RESULT;

        Ok(())
    }

    pub fn request_randomness(
        ctx: Context<RequestRandomness>,
        permission_bump: u8,
        switchboard_state_bump: u8,
        guess: u8,
    ) -> Result<()> {
        // Transfer SOL to the vault.
        msg!("transferring sol to vault");
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.sol_vault.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, AMOUNT)?;

        // load client state
        let mut game_state = ctx.accounts.game_state.load_mut()?;
        // Clone the 'bump' value from the client state.
        let bump = game_state.bump.clone();

        // Update client state with the guessed value and reset result.
        game_state.guess = guess;
        game_state.result = 0;
        drop(game_state);

        let switchboard_program = ctx.accounts.switchboard_program.to_account_info();

        // Accounts for switchboard request randomness
        let vrf_request_randomness = VrfRequestRandomness {
            authority: ctx.accounts.game_state.to_account_info(),
            vrf: ctx.accounts.vrf.to_account_info(),
            oracle_queue: ctx.accounts.oracle_queue.to_account_info(),
            queue_authority: ctx.accounts.queue_authority.to_account_info(),
            data_buffer: ctx.accounts.data_buffer.to_account_info(),
            permission: ctx.accounts.permission.to_account_info(),
            escrow: ctx.accounts.escrow.clone(),
            payer_wallet: ctx.accounts.payer_wallet.clone(),
            payer_authority: ctx.accounts.player.to_account_info(),
            recent_blockhashes: ctx.accounts.recent_blockhashes.to_account_info(),
            program_state: ctx.accounts.program_state.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };

        // Prepare the signer seeds.
        let player_key = ctx.accounts.player.key();
        let signer_seeds: &[&[&[u8]]] = &[&[&GAME_SEED, player_key.as_ref(), &[bump]]];

        // Invoke CPI to request randomness from Switchboard.
        msg!("requesting randomness");
        vrf_request_randomness.invoke_signed(
            switchboard_program,
            switchboard_state_bump,
            permission_bump,
            signer_seeds,
        )?;

        msg!("randomness requested successfully");
        Ok(())
    }

    pub fn consume_randomness(ctx: Context<ConsumeRandomness>) -> Result<()> {
        // Load the Verifiable Random Function (VRF) account.
        let vrf = ctx.accounts.vrf.load()?;

        // Retrieve the randomness result from the VRF account.
        let result_buffer = vrf.get_result()?;

        // If the result buffer is empty (contains only zeros), there is no new randomness to consume, hence exit.
        if result_buffer == [0u8; 32] {
            msg!("VRF buffer is empty. Exiting...");
            return Ok(());
        }

        // Load the client's state.
        let game_state = &mut ctx.accounts.game_state.load_mut()?;
        let max_result = game_state.max_result;

        // If the new result buffer is the same as the stored result buffer, no new randomness has been generated.
        // So, there is nothing to update, hence exit.
        if result_buffer == game_state.result_buffer {
            msg!("Result buffer is unchanged. Exiting...");
            return Ok(());
        }

        // Cast the result buffer to a u128 number and calculate the new result.
        msg!("Result buffer is {:?}", result_buffer);
        let value: &[u128] = bytemuck::cast_slice(&result_buffer[..]);
        msg!("u128 buffer {:?}", value);
        let result = value[0] % max_result as u128 + 1;

        // Log the newly calculated result and the current guess.
        msg!(
            "Result Range [1 - {}], Result Value = {}, Current Guess = {}",
            max_result,
            result,
            game_state.guess
        );

        // If the client's guess is correct, transfer a certain amount of SOL to the player's account.
        if game_state.guess == result as u8 {
            let seeds = VAULT_SEED;
            let bump = *ctx.bumps.get("sol_vault").unwrap();
            let signer: &[&[&[u8]]] = &[&[seeds, &[bump]]];
            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.sol_vault.to_account_info(),
                        to: ctx.accounts.player.to_account_info(),
                    },
                    signer,
                ),
                AMOUNT.checked_mul(2).unwrap(),
            )?;
        }

        // Update the client's state.
        game_state.result_buffer = result_buffer;
        game_state.result = result;
        game_state.timestamp = clock::Clock::get().unwrap().unix_timestamp;

        Ok(())
    }

    pub fn close(_ctx: Context<Close>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    // switchboard
    // this is the account that will hold the VRF state (randomness)
    #[account(
        init,
        seeds = [
            GAME_SEED,
            player.key().as_ref(),
            ],
            payer = player,
            space = 8 + std::mem::size_of::<GameState>(),
            bump,
        )]
    pub game_state: AccountLoader<'info, GameState>,
    #[account(
            constraint = vrf.load()?.authority == game_state.key() @ ErrorCode::InvalidVrfAuthorityError
        )]
    pub vrf: AccountLoader<'info, VrfAccountData>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RequestRandomness<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
    )]
    pub sol_vault: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [
            GAME_SEED,
            player.key().as_ref(),
        ],
        bump = game_state.load()?.bump,
        has_one = vrf @ ErrorCode::InvalidVrfAccount
    )]
    pub game_state: AccountLoader<'info, GameState>,

    // SWITCHBOARD ACCOUNTS
    #[account(mut,
        has_one = escrow
    )]
    pub vrf: AccountLoader<'info, VrfAccountData>,
    #[account(mut,
        has_one = data_buffer
    )]
    pub oracle_queue: AccountLoader<'info, OracleQueueAccountData>,
    /// CHECK:
    #[account(mut,
        constraint =
            oracle_queue.load()?.authority == queue_authority.key()
    )]
    pub queue_authority: UncheckedAccount<'info>,
    /// CHECK
    #[account(mut)]
    pub data_buffer: UncheckedAccount<'info>,
    #[account(mut)]
    pub permission: AccountLoader<'info, PermissionAccountData>,
    #[account(mut,
        constraint =
            escrow.owner == program_state.key()
            && escrow.mint == program_state.load()?.token_mint
    )]
    pub escrow: Account<'info, TokenAccount>,
    #[account(mut)]
    pub program_state: AccountLoader<'info, SbState>,
    /// CHECK:
    #[account(
        address = *vrf.to_account_info().owner,
        constraint = switchboard_program.executable == true
    )]
    pub switchboard_program: UncheckedAccount<'info>,
    // PAYER ACCOUNTS
    #[account(mut,
        constraint =
            payer_wallet.owner == player.key()
            && escrow.mint == program_state.load()?.token_mint
    )]
    pub payer_wallet: Account<'info, TokenAccount>,

    /// CHECK:
    #[account(address = solana_program::sysvar::recent_blockhashes::ID)]
    pub recent_blockhashes: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ConsumeRandomness<'info> {
    pub player: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
    )]
    pub sol_vault: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [
            GAME_SEED,
            player.key().as_ref(),
        ],
        bump = game_state.load()?.bump,
        has_one = vrf @ ErrorCode::InvalidVrfAccount
    )]
    pub game_state: AccountLoader<'info, GameState>,
    pub vrf: AccountLoader<'info, VrfAccountData>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Close<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    #[account(
        mut,
        close = player,
        seeds = [
            GAME_SEED,
            player.key().as_ref(),
            ],
            bump,
        )]
    pub game_state: AccountLoader<'info, GameState>,
}

#[repr(packed)]
#[account(zero_copy(unsafe))]
#[derive(Default)]
pub struct GameState {
    pub guess: u8,
    pub bump: u8,
    pub max_result: u64,
    pub result_buffer: [u8; 32],
    pub result: u128,
    pub timestamp: i64,
    pub vrf: Pubkey,
}

#[error_code]
#[derive(Eq, PartialEq)]
pub enum ErrorCode {
    #[msg("Switchboard VRF Account's authority should be set to the client's state pubkey")]
    InvalidVrfAuthorityError,
    #[msg("Invalid VRF account provided.")]
    InvalidVrfAccount,
}
