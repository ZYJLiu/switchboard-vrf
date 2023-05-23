use anchor_lang::{
    prelude::*,
    solana_program::{clock, native_token::LAMPORTS_PER_SOL},
    system_program,
};
pub use anchor_spl::token::{Token, TokenAccount};
pub use switchboard_v2::{
    OracleQueueAccountData, PermissionAccountData, SbState, VrfAccountData, VrfRequestRandomness,
};

declare_id!("FXWi8jVNNcyCARo6JckMFPiqzcMhPo585NirdPvD2hva");

const GAME_SEED: &[u8] = b"GAME";
const VAULT_SEED: &[u8] = b"VAULT";
const AMOUNT: u64 = LAMPORTS_PER_SOL / 10;
const MAX_RESULT: u64 = 2;

#[program]
pub mod vrf {
    use super::*;

    // Instruction to initialize the game state account for new player
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let game_state = &mut ctx.accounts.game_state;

        // The bump for the game state program-derived address is stored to the game state.
        game_state.bump = ctx.bumps.get("game_state").unwrap().clone();

        // The Switchboard VRF account key is saved to the game state.
        // The VRF account is used by the VRF Oracle to store the requested randomness result.
        game_state.vrf = ctx.accounts.vrf.key();

        // The maximum possible game result value is set (Coin flip, result either 1 or 2).
        // Used to calculate the result of from the randomness result from the VRF Oracle stored to the VRF Account.
        game_state.max_result = MAX_RESULT;
        Ok(())
    }

    // Instruction to request randomness from the VRF Oracle and transfer SOL to the game's SOL vault.
    pub fn request_randomness(
        ctx: Context<RequestRandomness>,
        permission_bump: u8,
        switchboard_state_bump: u8,
        guess: u8, // The player's guess (1 or 2)
    ) -> Result<()> {
        // Transfer a predefined amount of SOL from the player to the game's SOL vault.
        msg!("transferring sol to vault");
        let cpi_context = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.sol_vault.to_account_info(),
            },
        );
        system_program::transfer(cpi_context, AMOUNT)?;

        // Get the game state account.
        let game_state = &mut ctx.accounts.game_state;
        // Clone the 'bump' value from the game state.
        let bump = game_state.bump.clone();

        // Update game state with the player's guessed value and reset result.
        game_state.guess = guess;

        // Reset the result to 0, we'll use this in the frontend to check when a new request has been made.
        // ie. if the result gets reset to 0, it indicates a new request has been made.
        game_state.result = 0;

        // Get the switchboard_program account for use in the CPI call.
        let switchboard_program = ctx.accounts.switchboard_program.to_account_info();

        // Accounts for Switchboard's request randomness instruction.
        let vrf_request_randomness = VrfRequestRandomness {
            // The game state PDA is the authority for the VRF Account.
            authority: ctx.accounts.game_state.to_account_info(),
            // VRF account where the new randomness result will be stored by the VRF Oracle.
            vrf: ctx.accounts.vrf.to_account_info(),
            // Switchboard VRF Oracle's queue account.
            oracle_queue: ctx.accounts.oracle_queue.to_account_info(),
            // Switchboard VRF Oracle's queue account authority.
            queue_authority: ctx.accounts.queue_authority.to_account_info(),
            // VRF account's data buffer account.
            data_buffer: ctx.accounts.data_buffer.to_account_info(),
            // VRF account's permission account (not entirely sure what this means)
            permission: ctx.accounts.permission.to_account_info(),
            // VRF account's "escrow" Wrapped SOL account that player funds
            // Funds from this account are used to pay Oracle for fulfilling the randomness request.
            escrow: ctx.accounts.escrow.clone(),
            // The player's Wrapped SOL account used to pay for the randomness request and fund the escrow.
            payer_wallet: ctx.accounts.payer_wallet.clone(),
            // The player's account requesting randomness.
            payer_authority: ctx.accounts.player.to_account_info(),
            recent_blockhashes: ctx.accounts.recent_blockhashes.to_account_info(),
            program_state: ctx.accounts.program_state.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };

        // Prepare the signer seeds. The game state PDA is used to "sign" for the CPI
        // In this demo, the game state PDA is the authority for the VRF account.
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

    // A "callback" instruction to consume the randomness result from the VRF Oracle.
    // This instruction is not invoked directly by the client, but rather by the VRF Oracle after fulfilling the randomness request.
    // The details for how to invoke this instruction is stored directly in the VRF account when creating the VRF account.
    // This is how the VRF Oracle knows how to invoke this instruction.
    pub fn consume_randomness(ctx: Context<ConsumeRandomness>) -> Result<()> {
        // Load the VRF account.
        let vrf = ctx.accounts.vrf.load()?;

        // Retrieve the randomness result stored on the VRF account.
        let result_buffer = vrf.get_result()?;

        // If the result buffer is empty (contains only zeros), there is no randomness to consume, hence exit.
        if result_buffer == [0u8; 32] {
            msg!("VRF buffer is empty. Exiting...");
            return Ok(());
        }

        // Load the game's state data.
        let game_state = &mut ctx.accounts.game_state;
        let max_result = game_state.max_result;

        // If the result buffer on the VRF account is the same as the result buffer stored on the game state account, then no new randomness has been generated.
        // So, there is nothing to update, hence exit.
        if result_buffer == game_state.result_buffer {
            msg!("Result buffer is unchanged. Exiting...");
            return Ok(());
        }

        // Cast the new result buffer to a u128 number and calculate the new result.
        msg!("Result buffer is {:?}", result_buffer);
        let value: &[u128] = bytemuck::cast_slice(&result_buffer[..]);
        msg!("u128 buffer {:?}", value);

        // Calculate the new result using modulo of the value and the max result value.
        // The result is a number between 1 and the max result value.
        let result = value[0] % max_result as u128 + 1;

        // Log the newly calculated result and the current guess.
        msg!(
            "Result Range [1 - {}], Result Value = {}, Current Guess = {}",
            max_result,
            result,
            game_state.guess
        );

        // If the client's guess is correct, transfer SOL from the game's SOL vault to the player's account.
        if game_state.guess == result as u8 {
            let bump = *ctx.bumps.get("sol_vault").unwrap();
            let signer: &[&[&[u8]]] = &[&[VAULT_SEED, &[bump]]];
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

        // Update the player's game state account.
        game_state.result_buffer = result_buffer;
        game_state.result = result;
        game_state.timestamp = clock::Clock::get().unwrap().unix_timestamp;

        Ok(())
    }

    // Instruction to close the game state account.
    // Used to reset when testing since each player can only have one game state account.
    // It does not close the VRF Account, which means the SOL from VRF Account will be lost.
    pub fn close(_ctx: Context<Close>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    // The player creating the game state account.
    #[account(mut)]
    pub player: Signer<'info>,

    // The game state account to be initialized.
    // The player's pubkey is used as a seed to derive PDA for the game state account.
    // This means each player can only have one game state account.
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
    pub game_state: Account<'info, GameState>,

    // The Switchboard VRF account used by the VRF Oracle to store new requested randomness result.
    // This account must be created prior to initializing the game state account.
    // For this demo, the authority of the vrf account is set to the game state account.
    // The authority is set using the game state account's PDA before actually initializing the game state account.
    // This means the only the game state account can be used to "request randomness" from the VRF Oracle.
    #[account(
        // The "authority" field of the VRF account must match the pubkey of the game state account.
        constraint = vrf.load()?.authority == game_state.key() @ ErrorCode::InvalidVrfAuthorityError
    )]
    pub vrf: AccountLoader<'info, VrfAccountData>,

    // System program account is required when creating new accounts.
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RequestRandomness<'info> {
    // The player requesting randomness.
    #[account(mut)]
    pub player: Signer<'info>,

    // The game's SOL vault account.
    // The player's SOL is transferred to this account when requesting randomness.
    #[account(
        mut,
        seeds = [VAULT_SEED],
        bump,
    )]
    pub sol_vault: SystemAccount<'info>,

    // The player's game state account.
    #[account(
        mut,
        seeds = [
            GAME_SEED,
            player.key().as_ref(),
        ],
        bump,
        // The VRF account provided to this instruction must match the VRF account stored on the game state account.
        has_one = vrf @ ErrorCode::InvalidVrfAccount
    )]
    pub game_state: Account<'info, GameState>,

    // The VRF account used by the VRF Oracle to store new requested randomness result.
    #[account(mut,
        // The escrow account provided to this instruction must match the escrow account stored on the VRF account.
        has_one = escrow
    )]
    pub vrf: AccountLoader<'info, VrfAccountData>,

    #[account(mut,
        // The data_buffer provided to this instruction must match the data_buffer account stored on the oracle queue account.
        has_one = data_buffer
    )]
    pub oracle_queue: AccountLoader<'info, OracleQueueAccountData>,

    /// CHECK:
    #[account(mut,
        // The queue_authority provided to this instruction must match the queue_authority account stored on the oracle queue account.
        constraint =
            oracle_queue.load()?.authority == queue_authority.key()
    )]
    pub queue_authority: UncheckedAccount<'info>,

    /// CHECK:
    #[account(mut)]
    pub data_buffer: UncheckedAccount<'info>,

    #[account(mut)]
    pub permission: AccountLoader<'info, PermissionAccountData>,

    // The "escrow" account is used to pay the VRF Oracle for fulfilling the randomness request.
    // Wrapped SOL is used as the payment token
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

    // Wrapped SOL account used to pay for the randomness request and fund the escrow.
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
        bump,
        has_one = vrf @ ErrorCode::InvalidVrfAccount
    )]
    pub game_state: Account<'info, GameState>,
    pub vrf: AccountLoader<'info, VrfAccountData>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Close<'info> {
    // The player closing the game state account.
    #[account(mut)]
    pub player: Signer<'info>,

    // The game state account to be closed.
    #[account(
        mut,
        close = player,
        seeds = [
            GAME_SEED,
            player.key().as_ref(),
        ],
        bump,
    )]
    pub game_state: Account<'info, GameState>,
}

// This struct represents the state of the game.
#[account]
pub struct GameState {
    // The guess made by the player in the game (1 or 2 representing heads or tails).
    pub guess: u8,
    // The 'bump' for the PDA of the game state account.
    pub bump: u8,
    // The maximum value that can be the result of the game.
    pub max_result: u64,
    // Buffer to copy the randomness result from the VRF Account after each successful request.
    pub result_buffer: [u8; 32],
    // The result calculated from the result_buffer (1 or 2 representing heads or tails).
    pub result: u128,
    // The timestamp of when the game state was last updated (in the consume randomness callback instruction).
    pub timestamp: i64,
    // The public key of the VRF account used created for this game state account.
    pub vrf: Pubkey,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Switchboard VRF Account's authority should be set to the client's state pubkey")]
    InvalidVrfAuthorityError,
    #[msg("Invalid VRF account provided.")]
    InvalidVrfAccount,
}
