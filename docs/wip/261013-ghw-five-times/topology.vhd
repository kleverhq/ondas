-- SPDX-License-Identifier: MIT
library ieee;
use ieee.std_logic_1164.all;
use std.env.all;
entity top is
  generic (pad_count : positive := 16; tick_count : positive := 256;
           dense_noise : boolean := false);
end;
architecture test of top is
  signal constant_probe : std_ulogic := '1';
  signal noise : std_ulogic_vector(pad_count-1 downto 0) := (others => '0');
begin
  probes : for i in 0 to 63 generate
    signal probe : std_ulogic := '0';
  begin
    process begin
      for tick in 1 to tick_count loop
        wait for 1 fs;
        if tick mod 64 = 1 then probe <= not probe; end if;
      end loop;
      wait;
    end process;
  end generate;
  process begin
    for tick in 1 to tick_count loop
      wait for 1 fs;
      if dense_noise or tick mod 64 = 1 then noise <= not noise; end if;
    end loop;
    wait for 1 fs; stop; wait;
  end process;
end;
