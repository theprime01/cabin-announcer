// Prints field offsets in the PMDG SDK structs. Build with tools\pmdg-offsets.ps1.
#include <stdio.h>
#include <stddef.h>
#include "PMDG_NG3_SDK.h"
#include "PMDG_777X_SDK.h"
#define O(s, f) printf("%s.%s = %u\n", #s, #f, (unsigned)offsetof(struct s, f))
int main(void) {
    printf("PMDG_NG3_Data size = %u\n", (unsigned)sizeof(struct PMDG_NG3_Data));
    O(PMDG_NG3_Data, COMM_NoSmokingSelector);
    O(PMDG_NG3_Data, COMM_FastenBeltsSelector);
    O(PMDG_NG3_Data, COMM_Attend_PressCount);
    O(PMDG_NG3_Data, DOOR_annunFWD_ENTRY);
    O(PMDG_NG3_Data, DOOR_annunFWD_SERVICE);
    O(PMDG_NG3_Data, DOOR_annunAIRSTAIR);
    O(PMDG_NG3_Data, DOOR_annunAFT_ENTRY);
    O(PMDG_NG3_Data, DOOR_annunAFT_SERVICE);
    O(PMDG_NG3_Data, FMC_CruiseAlt);
    O(PMDG_NG3_Data, FMC_DistanceToTOD);
    O(PMDG_NG3_Data, FMC_DistanceToDest);
    O(PMDG_NG3_Data, FMC_flightNumber);
    O(PMDG_NG3_Data, AircraftModel);
    printf("PMDG_777X_Data size = %u\n", (unsigned)sizeof(struct PMDG_777X_Data));
    O(PMDG_777X_Data, SIGNS_NoSmokingSelector);
    O(PMDG_777X_Data, SIGNS_SeatBeltsSelector);
    O(PMDG_777X_Data, DOOR_state);
    O(PMDG_777X_Data, FMC_CruiseAlt);
    O(PMDG_777X_Data, FMC_DistanceToTOD);
    O(PMDG_777X_Data, FMC_DistanceToDest);
    O(PMDG_777X_Data, FMC_flightNumber);
    O(PMDG_777X_Data, AircraftModel);
    return 0;
}
